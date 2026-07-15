//! Anthropic Messages adapter (`POST /messages` + `GET /models`).

use super::sse_events::{decode_sse_data, EventDecodeState};
use crate::adapter::{ChatStream, ProtocolAdapter};
use crate::auth::{apply_api_key_query, merge_extra_headers};
use crate::error::{map_http_error, ErrorBodyFamily, ProtocolError, ProtocolErrorKind};
use crate::sse::SseParser;
use crate::types::{
    join_api_path, ChatRequest, ChatRole, ModelInfo, ProtocolCapabilities, ProtocolKind,
    ProviderEndpoint, StreamEvent,
};
use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE};
use reqwest::{Client, Response};
use serde::Serialize;
use serde_json::Value;
use std::time::Duration;
use tokio::time::timeout;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const LIST_TIMEOUT: Duration = Duration::from_secs(30);
/// Stream idle timeout (no body bytes): model-protocol-sdk §5.
pub const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(120);
/// Anthropic requires `max_tokens`; used when ChatRequest omits it.
const DEFAULT_MAX_TOKENS: u32 = 4096;
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Anthropic Messages adapter (SSE streaming).
#[derive(Debug, Clone)]
pub struct AnthropicMessagesAdapter {
    client: Client,
    stream_idle_timeout: Duration,
}

impl AnthropicMessagesAdapter {
    /// Create an adapter with default HTTP timeouts.
    pub fn new() -> Result<Self, ProtocolError> {
        Self::with_stream_idle_timeout(STREAM_IDLE_TIMEOUT)
    }

    /// Create an adapter with a custom stream idle timeout (tests / specialized hosts).
    pub fn with_stream_idle_timeout(stream_idle_timeout: Duration) -> Result<Self, ProtocolError> {
        let client = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .map_err(|err| {
                ProtocolError::new(
                    ProtocolErrorKind::Transport,
                    format!("创建 HTTP 客户端失败: {err}"),
                )
            })?;
        Ok(Self {
            client,
            stream_idle_timeout,
        })
    }

    fn auth_headers(
        endpoint: &ProviderEndpoint,
        accept_event_stream: bool,
    ) -> Result<HeaderMap, ProtocolError> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Ok(v) = HeaderValue::from_str(ANTHROPIC_VERSION) {
            headers.insert(HeaderName::from_static("anthropic-version"), v);
        }
        if accept_event_stream {
            if let Ok(v) = HeaderValue::from_str("text/event-stream") {
                headers.insert(reqwest::header::ACCEPT, v);
            }
        }
        if !endpoint.api_key.is_empty() {
            if let Ok(v) = HeaderValue::from_str(endpoint.api_key.expose_secret()) {
                headers.insert(HeaderName::from_static("x-api-key"), v);
            }
        }
        merge_extra_headers(&mut headers, &endpoint.auth)?;
        Ok(headers)
    }

    async fn map_error_response(response: Response) -> ProtocolError {
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let bytes = response.bytes().await.unwrap_or_default();
        map_http_error(status, &headers, &bytes, ErrorBodyFamily::Anthropic)
    }
}

impl Default for AnthropicMessagesAdapter {
    fn default() -> Self {
        Self::new().expect("reqwest client")
    }
}

#[async_trait]
impl ProtocolAdapter for AnthropicMessagesAdapter {
    fn kind(&self) -> ProtocolKind {
        ProtocolKind::AnthropicMessages
    }

    fn capabilities(&self) -> ProtocolCapabilities {
        ProtocolCapabilities {
            streaming: true,
            list_models: true,
            reasoning_output: true,
            reasoning_control: true,
            tools: true,
            vision_input: true,
            stream_usage: true,
            custom_headers: true,
            api_key_query: true,
        }
    }

    async fn list_models(
        &self,
        endpoint: &ProviderEndpoint,
    ) -> Result<Vec<ModelInfo>, ProtocolError> {
        if endpoint.protocol != ProtocolKind::AnthropicMessages {
            return Err(ProtocolError::new(
                ProtocolErrorKind::Unsupported,
                "协议不受支持",
            ));
        }

        let url = apply_api_key_query(join_api_path(&endpoint.base_url, "models")?, endpoint);
        let request = self
            .client
            .get(url)
            .headers(Self::auth_headers(endpoint, false)?);

        let response = timeout(LIST_TIMEOUT, request.send())
            .await
            .map_err(|_| {
                ProtocolError::new(ProtocolErrorKind::Timeout, "模型列表请求超时")
                    .with_retryable(true)
            })?
            .map_err(map_reqwest_error)?;

        if !response.status().is_success() {
            return Err(Self::map_error_response(response).await);
        }

        let body = timeout(LIST_TIMEOUT, response.bytes())
            .await
            .map_err(|_| ProtocolError::new(ProtocolErrorKind::Timeout, "读取模型列表超时"))?
            .map_err(map_reqwest_error)?;

        parse_models_body(&body)
    }

    async fn stream_chat(
        &self,
        endpoint: &ProviderEndpoint,
        request: ChatRequest,
    ) -> Result<ChatStream, ProtocolError> {
        if endpoint.protocol != ProtocolKind::AnthropicMessages {
            return Err(ProtocolError::new(
                ProtocolErrorKind::Unsupported,
                "协议不受支持",
            ));
        }
        request.validate()?;
        request.check_capabilities(&self.capabilities())?;

        let url = apply_api_key_query(join_api_path(&endpoint.base_url, "messages")?, endpoint);
        let wire = WireMessagesRequest::try_from_request(&request)?;
        let http_request = self
            .client
            .post(url)
            .headers(Self::auth_headers(endpoint, true)?)
            .json(&wire);

        let response = http_request.send().await.map_err(map_reqwest_error)?;
        if !response.status().is_success() {
            return Err(Self::map_error_response(response).await);
        }

        let mut byte_stream = response.bytes_stream();
        let idle = self.stream_idle_timeout;
        let stream = async_stream::stream! {
            let mut parser = SseParser::new();
            let mut decode_state = EventDecodeState::default();
            let mut completed = false;
            let mut failed = false;

            // Dropping this stream cancels further polling and does not fabricate Completed (§3.8).
            while !completed && !failed {
                let next = timeout(idle, byte_stream.next()).await;
                match next {
                    Err(_) => {
                        failed = true;
                        yield Err(ProtocolError::new(
                            ProtocolErrorKind::Timeout,
                            "流式响应空闲超时",
                        )
                        .with_retryable(true));
                    }
                    Ok(Some(Ok(bytes))) => {
                        for sse_event in parser.push(&bytes) {
                            match decode_sse_data(&sse_event.data, &mut decode_state) {
                                Ok(events) => {
                                    for event in events {
                                        if matches!(event, StreamEvent::Completed { .. }) {
                                            completed = true;
                                        }
                                        yield Ok(event);
                                    }
                                }
                                Err(err) => {
                                    failed = true;
                                    yield Err(err);
                                    break;
                                }
                            }
                        }
                    }
                    Ok(Some(Err(err))) => {
                        failed = true;
                        yield Err(map_reqwest_error(err));
                    }
                    Ok(None) => {
                        if let Some(sse_event) = parser.finish() {
                            match decode_sse_data(&sse_event.data, &mut decode_state) {
                                Ok(events) => {
                                    for event in events {
                                        if matches!(event, StreamEvent::Completed { .. }) {
                                            completed = true;
                                        }
                                        yield Ok(event);
                                    }
                                }
                                Err(err) => {
                                    yield Err(err);
                                    break;
                                }
                            }
                        }
                        if !completed && !failed {
                            yield Err(ProtocolError::new(
                                ProtocolErrorKind::UnexpectedEof,
                                "流在完成前意外结束",
                            ));
                        }
                        break;
                    }
                }
            }
        };

        Ok(Box::pin(stream))
    }
}

fn map_reqwest_error(err: reqwest::Error) -> ProtocolError {
    if err.is_timeout() {
        return ProtocolError::new(ProtocolErrorKind::Timeout, "网络请求超时").with_retryable(true);
    }
    if err.is_connect() {
        return ProtocolError::new(ProtocolErrorKind::Transport, format!("连接失败: {err}"))
            .with_retryable(true);
    }
    ProtocolError::new(ProtocolErrorKind::Transport, format!("网络错误: {err}"))
}

fn parse_models_body(body: &[u8]) -> Result<Vec<ModelInfo>, ProtocolError> {
    let value: Value = serde_json::from_slice(body).map_err(|err| {
        ProtocolError::new(
            ProtocolErrorKind::Decode,
            format!("模型列表 JSON 无效: {err}"),
        )
    })?;
    let data = value
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or_else(|| ProtocolError::new(ProtocolErrorKind::Decode, "模型列表缺少 data 数组"))?;

    let mut models = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for item in data {
        let id = item
            .get("id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let Some(id) = id else {
            continue;
        };
        if !seen.insert(id.to_string()) {
            continue;
        }
        let display_name = item
            .get("display_name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(id)
            .to_string();
        models.push(ModelInfo {
            id: id.to_string(),
            display_name,
            owned_by: None,
        });
    }
    Ok(models)
}

#[derive(Debug, Serialize)]
struct WireMessagesRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<Value>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    /// Manual extended thinking (default V2 path). Adaptive is not auto-selected.
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<WireThinking>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<WireAnthropicTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<Value>,
}

#[derive(Debug, Serialize)]
struct WireThinking {
    #[serde(rename = "type")]
    thinking_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    budget_tokens: Option<u32>,
    /// V2 default when enabled so thinking_delta produces ReasoningDelta.
    #[serde(skip_serializing_if = "Option::is_none")]
    display: Option<String>,
}

#[derive(Debug, Serialize)]
struct WireAnthropicTool {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    input_schema: Value,
}

impl WireMessagesRequest {
    fn try_from_request(request: &ChatRequest) -> Result<Self, ProtocolError> {
        let max_tokens = request.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS);
        let thinking = encode_anthropic_thinking(request.reasoning.as_ref(), max_tokens)?;
        // Extended thinking + Required/Named tool_choice is rejected before HTTP (mapping §4).
        if thinking
            .as_ref()
            .map(|t| t.thinking_type == "enabled")
            .unwrap_or(false)
        {
            if let Some(choice) = &request.tool_choice {
                use crate::types::ToolChoice;
                if matches!(choice, ToolChoice::Required | ToolChoice::Named { .. }) {
                    return Err(ProtocolError::new(
                        ProtocolErrorKind::InvalidRequest,
                        "Anthropic extended thinking 启用时 tool_choice 仅允许 Auto 或 None",
                    ));
                }
            }
        }

        let mut system_parts = Vec::new();
        let mut messages = Vec::new();
        for m in &request.messages {
            match m.role {
                ChatRole::System => {
                    if !m.text_content().is_empty() {
                        system_parts.push(m.text_content());
                    }
                }
                ChatRole::User => {
                    messages.push(serde_json::json!({
                        "role": "user",
                        "content": encode_anthropic_content(m)?,
                    }));
                }
                ChatRole::Assistant => {
                    if m.tool_calls.is_empty() {
                        messages.push(serde_json::json!({
                            "role": "assistant",
                            "content": encode_anthropic_content(m)?,
                        }));
                    } else {
                        let mut blocks = encode_anthropic_content_blocks(m)?;
                        for tc in &m.tool_calls {
                            let input = serde_json::from_str::<Value>(&tc.arguments)
                                .unwrap_or_else(|_| Value::String(tc.arguments.clone()));
                            blocks.push(serde_json::json!({
                                "type": "tool_use",
                                "id": tc.id,
                                "name": tc.name,
                                "input": input,
                            }));
                        }
                        messages.push(serde_json::json!({
                            "role": "assistant",
                            "content": blocks,
                        }));
                    }
                }
                ChatRole::Tool => {
                    let tool_use_id = m
                        .tool_call_id
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .ok_or_else(|| {
                            ProtocolError::new(
                                ProtocolErrorKind::InvalidRequest,
                                "Tool 消息必须提供 tool_call_id",
                            )
                        })?;
                    // Anthropic: tool_result lives inside a user message.
                    messages.push(serde_json::json!({
                        "role": "user",
                        "content": [{
                            "type": "tool_result",
                            "tool_use_id": tool_use_id,
                            "content": m.text_content(),
                        }],
                    }));
                }
            }
        }
        let system = if system_parts.is_empty() {
            None
        } else {
            Some(system_parts.join("\n\n"))
        };
        let tools = encode_anthropic_tools(request.tools.as_ref());
        let tool_choice =
            encode_anthropic_tool_choice(request.tool_choice.as_ref(), tools.is_some());
        Ok(Self {
            model: request.model.trim().to_string(),
            max_tokens,
            messages,
            stream: true,
            system,
            temperature: request.temperature,
            thinking,
            tools,
            tool_choice,
        })
    }
}

/// mapping-v2 §5: Text / ImageBase64 / ImageUrl content blocks.
/// Pure single-text messages stay string-shaped for V1 compatibility.
fn encode_anthropic_content(message: &crate::types::ChatMessage) -> Result<Value, ProtocolError> {
    use crate::types::ContentPart;
    let has_image = message.parts.iter().any(|p| {
        matches!(
            p,
            ContentPart::ImageUrl { .. } | ContentPart::ImageBase64 { .. }
        )
    });
    let multi_text = message
        .parts
        .iter()
        .filter(|p| matches!(p, ContentPart::Text { .. }))
        .count()
        > 1;
    if !has_image && !multi_text {
        return Ok(Value::String(message.text_content()));
    }
    Ok(Value::Array(encode_anthropic_content_blocks(message)?))
}

fn encode_anthropic_content_blocks(
    message: &crate::types::ChatMessage,
) -> Result<Vec<Value>, ProtocolError> {
    use crate::types::ContentPart;
    let mut blocks = Vec::with_capacity(message.parts.len());
    for part in &message.parts {
        match part {
            ContentPart::Text { text } => {
                if text.is_empty() {
                    continue;
                }
                blocks.push(serde_json::json!({
                    "type": "text",
                    "text": text,
                }));
            }
            ContentPart::ImageBase64 { media_type, data } => {
                let data: String = data.chars().filter(|c| !c.is_whitespace()).collect();
                blocks.push(serde_json::json!({
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": media_type.trim(),
                        "data": data,
                    }
                }));
            }
            ContentPart::ImageUrl { url, .. } => {
                blocks.push(serde_json::json!({
                    "type": "image",
                    "source": {
                        "type": "url",
                        "url": url,
                    }
                }));
            }
        }
    }
    if blocks.is_empty() && message.tool_calls.is_empty() {
        return Err(ProtocolError::new(
            ProtocolErrorKind::InvalidRequest,
            "消息 content 编码结果为空",
        ));
    }
    Ok(blocks)
}

fn encode_anthropic_tools(
    tools: Option<&Vec<crate::types::ToolDefinition>>,
) -> Option<Vec<WireAnthropicTool>> {
    tools.map(|tools| {
        tools
            .iter()
            .map(|t| WireAnthropicTool {
                name: t.name.trim().to_string(),
                description: t.description.clone(),
                input_schema: t.parameters.clone(),
            })
            .collect()
    })
}

fn encode_anthropic_tool_choice(
    choice: Option<&crate::types::ToolChoice>,
    has_tools: bool,
) -> Option<Value> {
    use crate::types::ToolChoice;
    match choice {
        None if has_tools => Some(serde_json::json!({ "type": "auto" })),
        None => None,
        Some(ToolChoice::Auto) => Some(serde_json::json!({ "type": "auto" })),
        Some(ToolChoice::None) => Some(serde_json::json!({ "type": "none" })),
        Some(ToolChoice::Required) => Some(serde_json::json!({ "type": "any" })),
        Some(ToolChoice::Named { name }) => Some(serde_json::json!({
            "type": "tool",
            "name": name,
        })),
    }
}

/// Manual extended thinking per mapping-v2 §3.1 / §3.3 / §3.4 (default path).
///
/// Adaptive + output_config.effort is not selected automatically in V2.
fn encode_anthropic_thinking(
    policy: Option<&crate::types::ReasoningPolicy>,
    max_tokens: u32,
) -> Result<Option<WireThinking>, ProtocolError> {
    use crate::types::ReasoningMode;
    let Some(policy) = policy else {
        return Ok(None);
    };
    match policy.mode {
        ReasoningMode::Auto => Ok(None),
        ReasoningMode::Off => Ok(Some(WireThinking {
            thinking_type: "disabled".into(),
            budget_tokens: None,
            display: None,
        })),
        ReasoningMode::On => {
            // budget takes precedence over effort when both present.
            let budget = if let Some(n) = policy.budget_tokens {
                n
            } else if let Some(effort) = policy.effort {
                effort_to_manual_budget(effort)
            } else {
                4096
            };
            if budget > 0 && budget < 1024 {
                return Err(ProtocolError::new(
                    ProtocolErrorKind::InvalidRequest,
                    "Anthropic budget_tokens 必须 ≥ 1024（或为 0 仅用于关闭路径）",
                ));
            }
            if budget == 0 {
                return Err(ProtocolError::new(
                    ProtocolErrorKind::InvalidRequest,
                    "Anthropic On 模式 budget_tokens 不能为 0",
                ));
            }
            // Non-interleaved: budget must be < max_tokens.
            if budget >= max_tokens {
                return Err(ProtocolError::new(
                    ProtocolErrorKind::InvalidRequest,
                    format!(
                        "Anthropic budget_tokens ({budget}) 必须小于 max_tokens ({max_tokens})"
                    ),
                ));
            }
            Ok(Some(WireThinking {
                thinking_type: "enabled".into(),
                budget_tokens: Some(budget),
                display: Some("summarized".into()),
            }))
        }
    }
}

fn effort_to_manual_budget(effort: crate::types::ReasoningEffort) -> u32 {
    use crate::types::ReasoningEffort;
    match effort {
        ReasoningEffort::Minimal | ReasoningEffort::Low => 1024,
        ReasoningEffort::Medium => 4096,
        ReasoningEffort::High => 16384,
        ReasoningEffort::XHigh | ReasoningEffort::Max => 32000,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret::SecretString;
    use crate::types::{
        normalize_base_url, ChatMessage, ReasoningEffort, ReasoningMode, ReasoningPolicy,
    };

    #[test]
    fn kind_is_anthropic_messages() {
        let adapter = AnthropicMessagesAdapter::new().unwrap();
        assert_eq!(adapter.kind(), ProtocolKind::AnthropicMessages);
        let caps = adapter.capabilities();
        assert!(caps.reasoning_control);
        assert!(caps.reasoning_output);
        assert!(caps.tools);
        assert!(caps.vision_input);
        assert!(caps.stream_usage);
        assert!(caps.custom_headers);
        assert!(caps.api_key_query);
    }

    #[test]
    fn tools_wire_and_thinking_named_conflict() {
        use crate::types::{ToolChoice, ToolDefinition};
        let tools = vec![ToolDefinition {
            name: "lookup".into(),
            description: Some("d".into()),
            parameters: serde_json::json!({"type":"object"}),
        }];
        let ok = ChatRequest {
            model: "m".into(),
            messages: vec![
                ChatMessage::text(ChatRole::User, "hi"),
                ChatMessage {
                    role: ChatRole::Assistant,
                    parts: vec![],
                    tool_call_id: None,
                    tool_calls: vec![crate::types::ToolCall {
                        id: "tu_1".into(),
                        name: "lookup".into(),
                        arguments: r#"{"q":1}"#.into(),
                    }],
                },
                ChatMessage {
                    role: ChatRole::Tool,
                    parts: vec![crate::types::ContentPart::Text { text: "ok".into() }],
                    tool_call_id: Some("tu_1".into()),
                    tool_calls: vec![],
                },
            ],
            temperature: None,
            max_tokens: Some(8192),
            reasoning: None,
            tools: Some(tools.clone()),
            tool_choice: Some(ToolChoice::Auto),
        };
        let json =
            serde_json::to_value(WireMessagesRequest::try_from_request(&ok).unwrap()).unwrap();
        assert_eq!(json["tools"][0]["name"], "lookup");
        assert_eq!(json["tools"][0]["input_schema"]["type"], "object");
        assert_eq!(json["tool_choice"]["type"], "auto");
        assert_eq!(json["messages"][1]["content"][0]["type"], "tool_use");
        assert_eq!(json["messages"][2]["role"], "user");
        assert_eq!(json["messages"][2]["content"][0]["type"], "tool_result");

        let conflict = ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessage::text(ChatRole::User, "hi")],
            temperature: None,
            max_tokens: Some(8192),
            reasoning: Some(ReasoningPolicy {
                mode: ReasoningMode::On,
                effort: None,
                budget_tokens: Some(1024),
            }),
            tools: Some(tools),
            tool_choice: Some(ToolChoice::Named {
                name: "lookup".into(),
            }),
        };
        let err = WireMessagesRequest::try_from_request(&conflict).unwrap_err();
        assert_eq!(err.kind, ProtocolErrorKind::InvalidRequest);
    }

    #[test]
    fn wire_request_system_and_default_max_tokens() {
        let req = ChatRequest {
            model: "claude".into(),
            messages: vec![
                ChatMessage::text(ChatRole::System, "sys"),
                ChatMessage::text(ChatRole::User, "hi"),
            ],
            temperature: Some(0.5),
            max_tokens: None,
            reasoning: None,
            tools: None,
            tool_choice: None,
        };
        let wire = WireMessagesRequest::try_from_request(&req).unwrap();
        let json = serde_json::to_value(&wire).unwrap();
        assert_eq!(json["stream"], true);
        assert_eq!(json["max_tokens"], 4096);
        assert_eq!(json["system"], "sys");
        assert_eq!(json["temperature"], 0.5);
        assert_eq!(json["messages"].as_array().unwrap().len(), 1);
        assert_eq!(json["messages"][0]["role"], "user");
        assert!(json.get("tools").is_none());
        assert!(json.get("thinking").is_none());
    }

    #[test]
    fn wire_request_uses_explicit_max_tokens() {
        let req = ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessage::text(ChatRole::User, "hi")],
            temperature: None,
            max_tokens: Some(128),
            reasoning: None,
            tools: None,
            tool_choice: None,
        };
        let wire = WireMessagesRequest::try_from_request(&req).unwrap();
        let json = serde_json::to_value(&wire).unwrap();
        assert_eq!(json["max_tokens"], 128);
        assert!(json.get("system").is_none());
        assert!(json.get("temperature").is_none());
    }

    #[test]
    fn reasoning_none_auto_omit_off_budget_and_effort() {
        let base = |policy: Option<ReasoningPolicy>, max_tokens: Option<u32>| ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessage::text(ChatRole::User, "hi")],
            temperature: None,
            max_tokens,
            reasoning: policy,
            tools: None,
            tool_choice: None,
        };

        let none_json = serde_json::to_value(
            WireMessagesRequest::try_from_request(&base(None, Some(8192))).unwrap(),
        )
        .unwrap();
        assert!(none_json.get("thinking").is_none());

        let auto_json = serde_json::to_value(
            WireMessagesRequest::try_from_request(&base(
                Some(ReasoningPolicy {
                    mode: ReasoningMode::Auto,
                    effort: Some(ReasoningEffort::High),
                    budget_tokens: Some(2000),
                }),
                Some(8192),
            ))
            .unwrap(),
        )
        .unwrap();
        assert!(auto_json.get("thinking").is_none());

        let off_json = serde_json::to_value(
            WireMessagesRequest::try_from_request(&base(
                Some(ReasoningPolicy {
                    mode: ReasoningMode::Off,
                    effort: Some(ReasoningEffort::High),
                    budget_tokens: None,
                }),
                Some(8192),
            ))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(off_json["thinking"]["type"], "disabled");
        assert!(off_json["thinking"].get("budget_tokens").is_none());

        // budget < 1024 → InvalidRequest
        let err = WireMessagesRequest::try_from_request(&base(
            Some(ReasoningPolicy {
                mode: ReasoningMode::On,
                effort: None,
                budget_tokens: Some(1023),
            }),
            Some(8192),
        ))
        .unwrap_err();
        assert_eq!(err.kind, ProtocolErrorKind::InvalidRequest);

        // budget 1024 ok
        let ok_json = serde_json::to_value(
            WireMessagesRequest::try_from_request(&base(
                Some(ReasoningPolicy {
                    mode: ReasoningMode::On,
                    effort: None,
                    budget_tokens: Some(1024),
                }),
                Some(8192),
            ))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(ok_json["thinking"]["type"], "enabled");
        assert_eq!(ok_json["thinking"]["budget_tokens"], 1024);
        assert_eq!(ok_json["thinking"]["display"], "summarized");

        // On + none → default 4096; need max_tokens > 4096
        let def_json = serde_json::to_value(
            WireMessagesRequest::try_from_request(&base(
                Some(ReasoningPolicy {
                    mode: ReasoningMode::On,
                    effort: None,
                    budget_tokens: None,
                }),
                Some(8192),
            ))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(def_json["thinking"]["budget_tokens"], 4096);

        // effort → budget table; budget wins over effort
        let effort_json = serde_json::to_value(
            WireMessagesRequest::try_from_request(&base(
                Some(ReasoningPolicy {
                    mode: ReasoningMode::On,
                    effort: Some(ReasoningEffort::High),
                    budget_tokens: None,
                }),
                Some(20000),
            ))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(effort_json["thinking"]["budget_tokens"], 16384);

        let budget_wins = serde_json::to_value(
            WireMessagesRequest::try_from_request(&base(
                Some(ReasoningPolicy {
                    mode: ReasoningMode::On,
                    effort: Some(ReasoningEffort::High),
                    budget_tokens: Some(2048),
                }),
                Some(8192),
            ))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(budget_wins["thinking"]["budget_tokens"], 2048);

        // budget >= max_tokens → InvalidRequest
        let err = WireMessagesRequest::try_from_request(&base(
            Some(ReasoningPolicy {
                mode: ReasoningMode::On,
                effort: None,
                budget_tokens: Some(4096),
            }),
            Some(4096),
        ))
        .unwrap_err();
        assert_eq!(err.kind, ProtocolErrorKind::InvalidRequest);
    }

    #[test]
    fn multimodal_wire_image_blocks() {
        use crate::types::{ChatMessage, ContentPart};

        let text_only = ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessage::text(ChatRole::User, "hi")],
            temperature: None,
            max_tokens: None,
            reasoning: None,
            tools: None,
            tool_choice: None,
        };
        let json = serde_json::to_value(WireMessagesRequest::try_from_request(&text_only).unwrap())
            .unwrap();
        assert_eq!(json["messages"][0]["content"], "hi");

        let with_images = ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessage {
                role: ChatRole::User,
                parts: vec![
                    ContentPart::Text {
                        text: "look".into(),
                    },
                    ContentPart::ImageBase64 {
                        media_type: "image/png".into(),
                        data: "aGVsbG8=".into(),
                    },
                    ContentPart::ImageUrl {
                        url: "https://example.com/b.png".into(),
                        detail: None,
                    },
                ],
                tool_call_id: None,
                tool_calls: vec![],
            }],
            temperature: None,
            max_tokens: None,
            reasoning: None,
            tools: None,
            tool_choice: None,
        };
        let json =
            serde_json::to_value(WireMessagesRequest::try_from_request(&with_images).unwrap())
                .unwrap();
        let content = &json["messages"][0]["content"];
        assert!(content.is_array());
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "image");
        assert_eq!(content[1]["source"]["type"], "base64");
        assert_eq!(content[1]["source"]["media_type"], "image/png");
        assert_eq!(content[1]["source"]["data"], "aGVsbG8=");
        assert_eq!(content[2]["type"], "image");
        assert_eq!(content[2]["source"]["type"], "url");
        assert_eq!(content[2]["source"]["url"], "https://example.com/b.png");
        assert!(
            AnthropicMessagesAdapter::new()
                .unwrap()
                .capabilities()
                .vision_input
        );
    }

    #[test]
    fn parse_models_display_name_and_dedup() {
        let body = br#"{
            "data":[
                {"id":"a","display_name":"Alpha"},
                {"id":"a"},
                {"id":"  "},
                {"id":"b"}
            ]
        }"#;
        let models = parse_models_body(body).unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "a");
        assert_eq!(models[0].display_name, "Alpha");
        assert_eq!(models[1].id, "b");
        assert_eq!(models[1].display_name, "b");
    }

    #[test]
    fn auth_headers_x_api_key_and_version() {
        let base = normalize_base_url("http://127.0.0.1:9/v1").unwrap();
        let with_key = ProviderEndpoint {
            protocol: ProtocolKind::AnthropicMessages,
            base_url: base.clone(),
            api_key: SecretString::new("sk-test"),
            auth: Default::default(),
        };
        let headers = AnthropicMessagesAdapter::auth_headers(&with_key, true).unwrap();
        assert_eq!(
            headers.get("x-api-key").and_then(|v| v.to_str().ok()),
            Some("sk-test")
        );
        assert_eq!(
            headers
                .get("anthropic-version")
                .and_then(|v| v.to_str().ok()),
            Some(ANTHROPIC_VERSION)
        );
        assert!(headers.contains_key(reqwest::header::ACCEPT));
        assert!(!headers.contains_key(reqwest::header::AUTHORIZATION));

        let no_key = ProviderEndpoint {
            protocol: ProtocolKind::AnthropicMessages,
            base_url: base,
            api_key: SecretString::new(""),
            auth: Default::default(),
        };
        let headers = AnthropicMessagesAdapter::auth_headers(&no_key, false).unwrap();
        assert!(!headers.contains_key("x-api-key"));
        assert!(headers.contains_key("anthropic-version"));
    }

    #[test]
    fn parse_anthropic_error_uses_type_as_code() {
        let body = r#"{"type":"error","error":{"type":"authentication_error","message":"bad sk-secret-key"}}"#;
        let (msg, code) = crate::error::parse_anthropic_error_body(body);
        assert_eq!(code.as_deref(), Some("authentication_error"));
        assert!(msg.unwrap().contains("REDACTED"));
    }
}
