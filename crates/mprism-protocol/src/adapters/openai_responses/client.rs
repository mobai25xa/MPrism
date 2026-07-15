//! OpenAI Responses API adapter (`POST /responses` + `GET /models`).

use super::events::{decode_sse_data, EventDecodeState};
use crate::adapter::{ChatStream, ProtocolAdapter};
use crate::auth::{apply_api_key_query, merge_extra_headers};
use crate::error::{map_http_error, ErrorBodyFamily, ProtocolError, ProtocolErrorKind};
use crate::sse::SseParser;
use crate::types::{
    join_api_path, ChatRequest, ModelInfo, ProtocolCapabilities, ProtocolKind, ProviderEndpoint,
    StreamEvent,
};
use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use reqwest::{Client, Response};
use serde::Serialize;
use serde_json::Value;
use std::time::Duration;
use tokio::time::timeout;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const LIST_TIMEOUT: Duration = Duration::from_secs(30);
/// Stream idle timeout (no body bytes): model-protocol-sdk §5.
pub const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(120);

/// OpenAI Responses adapter (semantic SSE streaming).
#[derive(Debug, Clone)]
pub struct OpenAiResponsesAdapter {
    client: Client,
    stream_idle_timeout: Duration,
}

impl OpenAiResponsesAdapter {
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
        if accept_event_stream {
            if let Ok(v) = HeaderValue::from_str("text/event-stream") {
                headers.insert(reqwest::header::ACCEPT, v);
            }
        }
        if !endpoint.api_key.is_empty() {
            let value = format!("Bearer {}", endpoint.api_key.expose_secret());
            if let Ok(v) = HeaderValue::from_str(&value) {
                headers.insert(AUTHORIZATION, v);
            }
        }
        merge_extra_headers(&mut headers, &endpoint.auth)?;
        Ok(headers)
    }

    async fn map_error_response(response: Response) -> ProtocolError {
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let bytes = response.bytes().await.unwrap_or_default();
        map_http_error(status, &headers, &bytes, ErrorBodyFamily::OpenAi)
    }
}

impl Default for OpenAiResponsesAdapter {
    fn default() -> Self {
        Self::new().expect("reqwest client")
    }
}

#[async_trait]
impl ProtocolAdapter for OpenAiResponsesAdapter {
    fn kind(&self) -> ProtocolKind {
        ProtocolKind::OpenAiResponses
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
        if endpoint.protocol != ProtocolKind::OpenAiResponses {
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
        if endpoint.protocol != ProtocolKind::OpenAiResponses {
            return Err(ProtocolError::new(
                ProtocolErrorKind::Unsupported,
                "协议不受支持",
            ));
        }
        request.validate()?;
        request.check_capabilities(&self.capabilities())?;

        let url = apply_api_key_query(join_api_path(&endpoint.base_url, "responses")?, endpoint);
        let wire = WireResponseRequest::try_from_request(&request)?;
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
        let owned_by = item
            .get("owned_by")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        models.push(ModelInfo {
            id: id.to_string(),
            display_name: id.to_string(),
            owned_by,
        });
    }
    Ok(models)
}

#[derive(Debug, Serialize)]
struct WireResponseRequest {
    model: String,
    /// Easy message array + function_call / function_call_output items.
    input: Vec<Value>,
    stream: bool,
    store: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    /// Official `reasoning: { effort, summary? }`. budget_tokens is ignored.
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<WireReasoning>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<WireResponseTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<Value>,
}

#[derive(Debug, Serialize)]
struct WireReasoning {
    effort: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
}

#[derive(Debug, Serialize)]
struct WireResponseTool {
    #[serde(rename = "type")]
    tool_type: &'static str,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    parameters: Value,
}

impl WireResponseRequest {
    fn try_from_request(request: &ChatRequest) -> Result<Self, ProtocolError> {
        let reasoning = encode_responses_reasoning(request.reasoning.as_ref())?;
        let tools = encode_responses_tools(request.tools.as_ref());
        let tool_choice =
            encode_responses_tool_choice(request.tool_choice.as_ref(), tools.is_some());
        Ok(Self {
            model: request.model.trim().to_string(),
            input: encode_responses_input(&request.messages)?,
            stream: true,
            store: false,
            temperature: request.temperature,
            max_output_tokens: request.max_tokens,
            reasoning,
            tools,
            tool_choice,
        })
    }
}

fn encode_responses_tools(
    tools: Option<&Vec<crate::types::ToolDefinition>>,
) -> Option<Vec<WireResponseTool>> {
    tools.map(|tools| {
        tools
            .iter()
            .map(|t| WireResponseTool {
                tool_type: "function",
                name: t.name.trim().to_string(),
                description: t.description.clone(),
                parameters: t.parameters.clone(),
            })
            .collect()
    })
}

fn encode_responses_tool_choice(
    choice: Option<&crate::types::ToolChoice>,
    has_tools: bool,
) -> Option<Value> {
    use crate::types::ToolChoice;
    match choice {
        None if has_tools => Some(Value::String("auto".into())),
        None => None,
        Some(ToolChoice::Auto) => Some(Value::String("auto".into())),
        Some(ToolChoice::None) => Some(Value::String("none".into())),
        Some(ToolChoice::Required) => Some(Value::String("required".into())),
        Some(ToolChoice::Named { name }) => Some(serde_json::json!({
            "type": "function",
            "name": name,
        })),
    }
}

fn encode_responses_input(
    messages: &[crate::types::ChatMessage],
) -> Result<Vec<Value>, ProtocolError> {
    use crate::types::ChatRole;
    let mut input = Vec::new();
    for m in messages {
        match m.role {
            ChatRole::Tool => {
                let call_id = m
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
                input.push(serde_json::json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": m.text_content(),
                }));
            }
            ChatRole::Assistant if !m.tool_calls.is_empty() => {
                if let Some(content) = encode_responses_message_content(m)? {
                    input.push(serde_json::json!({
                        "role": "assistant",
                        "content": content,
                    }));
                }
                for tc in &m.tool_calls {
                    input.push(serde_json::json!({
                        "type": "function_call",
                        "call_id": tc.id,
                        "name": tc.name,
                        "arguments": tc.arguments,
                    }));
                }
            }
            role => {
                let content = encode_responses_message_content(m)?.ok_or_else(|| {
                    ProtocolError::new(
                        ProtocolErrorKind::InvalidRequest,
                        "消息 content 编码结果为空",
                    )
                })?;
                input.push(serde_json::json!({
                    "role": role.as_str(),
                    "content": content,
                }));
            }
        }
    }
    Ok(input)
}

/// mapping-v2 §5: Text → string or input_text parts; images → input_image (URL/data URL).
fn encode_responses_message_content(
    message: &crate::types::ChatMessage,
) -> Result<Option<Value>, ProtocolError> {
    use crate::types::ContentPart;
    if !responses_needs_multipart(&message.parts) {
        let text = message.text_content();
        if text.is_empty() {
            return Ok(None);
        }
        return Ok(Some(Value::String(text)));
    }

    let mut parts = Vec::with_capacity(message.parts.len());
    for part in &message.parts {
        match part {
            ContentPart::Text { text } => {
                if text.is_empty() {
                    continue;
                }
                parts.push(serde_json::json!({
                    "type": "input_text",
                    "text": text,
                }));
            }
            ContentPart::ImageUrl { url, detail } => {
                let mut obj = serde_json::Map::new();
                obj.insert("type".into(), Value::String("input_image".into()));
                obj.insert("image_url".into(), Value::String(url.clone()));
                if let Some(d) = detail.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    obj.insert("detail".into(), Value::String(d.to_string()));
                }
                parts.push(Value::Object(obj));
            }
            ContentPart::ImageBase64 { media_type, data } => {
                let data_url = format_data_url(media_type, data);
                parts.push(serde_json::json!({
                    "type": "input_image",
                    "image_url": data_url,
                }));
            }
        }
    }
    if parts.is_empty() {
        return Ok(None);
    }
    Ok(Some(Value::Array(parts)))
}

fn responses_needs_multipart(parts: &[crate::types::ContentPart]) -> bool {
    use crate::types::ContentPart;
    let has_image = parts.iter().any(|p| {
        matches!(
            p,
            ContentPart::ImageUrl { .. } | ContentPart::ImageBase64 { .. }
        )
    });
    if has_image {
        return true;
    }
    parts
        .iter()
        .filter(|p| matches!(p, ContentPart::Text { .. }))
        .count()
        > 1
}

fn format_data_url(media_type: &str, data: &str) -> String {
    let media_type = media_type.trim();
    let data: String = data.chars().filter(|c| !c.is_whitespace()).collect();
    format!("data:{media_type};base64,{data}")
}

/// Encode Responses `reasoning` object per mapping-v2 §3.
fn encode_responses_reasoning(
    policy: Option<&crate::types::ReasoningPolicy>,
) -> Result<Option<WireReasoning>, ProtocolError> {
    use crate::types::{ReasoningEffort, ReasoningMode};
    let Some(policy) = policy else {
        return Ok(None);
    };
    match policy.mode {
        ReasoningMode::Auto => Ok(None),
        ReasoningMode::Off => Ok(Some(WireReasoning {
            effort: "none".into(),
            summary: None,
        })),
        ReasoningMode::On => {
            // budget_tokens is ignored on Responses.
            let effort = match policy.effort {
                None => "medium",
                Some(ReasoningEffort::Minimal) => "minimal",
                Some(ReasoningEffort::Low) => "low",
                Some(ReasoningEffort::Medium) => "medium",
                Some(ReasoningEffort::High) => "high",
                Some(ReasoningEffort::XHigh) => "xhigh",
                Some(ReasoningEffort::Max) => {
                    return Err(ProtocolError::new(
                        ProtocolErrorKind::Unsupported,
                        "OpenAI Responses 不支持 ReasoningEffort::Max（官方无 max）",
                    ));
                }
            };
            Ok(Some(WireReasoning {
                effort: effort.into(),
                summary: Some("auto".into()),
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret::SecretString;
    use crate::types::{
        normalize_base_url, ChatMessage, ChatRole, ReasoningEffort, ReasoningMode, ReasoningPolicy,
    };

    #[test]
    fn kind_is_openai_responses() {
        let adapter = OpenAiResponsesAdapter::new().unwrap();
        assert_eq!(adapter.kind(), ProtocolKind::OpenAiResponses);
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
    fn wire_request_stream_store_and_optionals() {
        let req = ChatRequest {
            model: "m".into(),
            messages: vec![
                ChatMessage::text(ChatRole::System, "sys"),
                ChatMessage::text(ChatRole::User, "hi"),
            ],
            temperature: Some(0.5),
            max_tokens: Some(128),
            reasoning: None,
            tools: None,
            tool_choice: None,
        };
        let wire = WireResponseRequest::try_from_request(&req).unwrap();
        let json = serde_json::to_value(&wire).unwrap();
        assert_eq!(json["stream"], true);
        assert_eq!(json["store"], false);
        assert_eq!(json["temperature"], 0.5);
        assert_eq!(json["max_output_tokens"], 128);
        assert!(json.get("max_tokens").is_none());
        assert!(json.get("reasoning").is_none());
        assert_eq!(json["input"].as_array().unwrap().len(), 2);
        assert_eq!(json["input"][0]["role"], "system");
        assert_eq!(json["input"][1]["role"], "user");
        assert!(json.get("tools").is_none());
    }

    #[test]
    fn tools_wire_and_function_call_input() {
        use crate::types::{ToolChoice, ToolDefinition};
        let req = ChatRequest {
            model: "m".into(),
            messages: vec![
                ChatMessage::text(ChatRole::User, "hi"),
                ChatMessage {
                    role: ChatRole::Assistant,
                    parts: vec![],
                    tool_call_id: None,
                    tool_calls: vec![crate::types::ToolCall {
                        id: "call_1".into(),
                        name: "lookup".into(),
                        arguments: r#"{"q":1}"#.into(),
                    }],
                },
                ChatMessage {
                    role: ChatRole::Tool,
                    parts: vec![crate::types::ContentPart::Text { text: "ok".into() }],
                    tool_call_id: Some("call_1".into()),
                    tool_calls: vec![],
                },
            ],
            temperature: None,
            max_tokens: None,
            reasoning: None,
            tools: Some(vec![ToolDefinition {
                name: "lookup".into(),
                description: None,
                parameters: serde_json::json!({"type":"object"}),
            }]),
            tool_choice: Some(ToolChoice::Required),
        };
        let json =
            serde_json::to_value(WireResponseRequest::try_from_request(&req).unwrap()).unwrap();
        assert_eq!(json["tools"][0]["type"], "function");
        assert_eq!(json["tools"][0]["name"], "lookup");
        assert_eq!(json["tool_choice"], "required");
        assert_eq!(json["input"][1]["type"], "function_call");
        assert_eq!(json["input"][1]["call_id"], "call_1");
        assert_eq!(json["input"][2]["type"], "function_call_output");
        assert_eq!(json["input"][2]["call_id"], "call_1");
        assert!(adapter_tools_cap());
    }

    fn adapter_tools_cap() -> bool {
        OpenAiResponsesAdapter::new().unwrap().capabilities().tools
    }

    #[test]
    fn wire_request_omits_optional_none() {
        let req = ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessage::text(ChatRole::User, "hi")],
            temperature: None,
            max_tokens: None,
            reasoning: None,
            tools: None,
            tool_choice: None,
        };
        let wire = WireResponseRequest::try_from_request(&req).unwrap();
        let json = serde_json::to_value(&wire).unwrap();
        assert_eq!(json["store"], false);
        assert!(json.get("temperature").is_none());
        assert!(json.get("max_output_tokens").is_none());
        assert!(json.get("tools").is_none());
        assert!(json.get("previous_response_id").is_none());
        assert!(json.get("reasoning").is_none());
    }

    #[test]
    fn reasoning_none_auto_omit_off_and_effort_table() {
        let base = |policy: Option<ReasoningPolicy>| ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessage::text(ChatRole::User, "hi")],
            temperature: None,
            max_tokens: None,
            reasoning: policy,
            tools: None,
            tool_choice: None,
        };

        let none_json =
            serde_json::to_value(WireResponseRequest::try_from_request(&base(None)).unwrap())
                .unwrap();
        assert!(none_json.get("reasoning").is_none());

        let auto_json = serde_json::to_value(
            WireResponseRequest::try_from_request(&base(Some(ReasoningPolicy {
                mode: ReasoningMode::Auto,
                effort: Some(ReasoningEffort::High),
                budget_tokens: Some(999),
            })))
            .unwrap(),
        )
        .unwrap();
        assert!(auto_json.get("reasoning").is_none());

        let off_json = serde_json::to_value(
            WireResponseRequest::try_from_request(&base(Some(ReasoningPolicy {
                mode: ReasoningMode::Off,
                effort: Some(ReasoningEffort::High),
                budget_tokens: Some(1),
            })))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(off_json["reasoning"]["effort"], "none");
        assert!(off_json["reasoning"].get("summary").is_none());

        let cases = [
            (None, "medium"),
            (Some(ReasoningEffort::Minimal), "minimal"),
            (Some(ReasoningEffort::Low), "low"),
            (Some(ReasoningEffort::Medium), "medium"),
            (Some(ReasoningEffort::High), "high"),
            (Some(ReasoningEffort::XHigh), "xhigh"),
        ];
        for (effort, expected) in cases {
            let json = serde_json::to_value(
                WireResponseRequest::try_from_request(&base(Some(ReasoningPolicy {
                    mode: ReasoningMode::On,
                    effort,
                    budget_tokens: Some(12345), // ignored
                })))
                .unwrap(),
            )
            .unwrap();
            assert_eq!(json["reasoning"]["effort"], expected, "effort={effort:?}");
            assert_eq!(json["reasoning"]["summary"], "auto");
            assert!(json["reasoning"].get("budget_tokens").is_none());
        }

        let max_err = WireResponseRequest::try_from_request(&base(Some(ReasoningPolicy {
            mode: ReasoningMode::On,
            effort: Some(ReasoningEffort::Max),
            budget_tokens: None,
        })))
        .unwrap_err();
        assert_eq!(max_err.kind, ProtocolErrorKind::Unsupported);
    }

    #[test]
    fn multimodal_wire_input_image_url_and_base64() {
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
        let json = serde_json::to_value(WireResponseRequest::try_from_request(&text_only).unwrap())
            .unwrap();
        assert_eq!(json["input"][0]["content"], "hi");

        let with_images = ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessage {
                role: ChatRole::User,
                parts: vec![
                    ContentPart::Text {
                        text: "describe".into(),
                    },
                    ContentPart::ImageUrl {
                        url: "https://example.com/a.png".into(),
                        detail: Some("low".into()),
                    },
                    ContentPart::ImageBase64 {
                        media_type: "image/jpeg".into(),
                        data: "aGVsbG8=".into(),
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
            serde_json::to_value(WireResponseRequest::try_from_request(&with_images).unwrap())
                .unwrap();
        let content = &json["input"][0]["content"];
        assert!(content.is_array());
        assert_eq!(content[0]["type"], "input_text");
        assert_eq!(content[0]["text"], "describe");
        assert_eq!(content[1]["type"], "input_image");
        assert_eq!(content[1]["image_url"], "https://example.com/a.png");
        assert_eq!(content[1]["detail"], "low");
        assert_eq!(content[2]["type"], "input_image");
        assert_eq!(content[2]["image_url"], "data:image/jpeg;base64,aGVsbG8=");
        assert!(
            OpenAiResponsesAdapter::new()
                .unwrap()
                .capabilities()
                .vision_input
        );
    }

    #[test]
    fn parse_models_dedup_and_skip_empty() {
        let body = br#"{
            "object":"list",
            "data":[
                {"id":"a","owned_by":"x"},
                {"id":"a"},
                {"id":"  "},
                {"id":"b"}
            ]
        }"#;
        let models = parse_models_body(body).unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "a");
        assert_eq!(models[1].id, "b");
    }

    #[test]
    fn auth_header_only_when_key_present() {
        let base = normalize_base_url("http://127.0.0.1:9/v1").unwrap();
        let with_key = ProviderEndpoint {
            protocol: ProtocolKind::OpenAiResponses,
            base_url: base.clone(),
            api_key: SecretString::new("sk-test"),
            auth: Default::default(),
        };
        let headers = OpenAiResponsesAdapter::auth_headers(&with_key, true).unwrap();
        assert!(headers.contains_key(AUTHORIZATION));

        let no_key = ProviderEndpoint {
            protocol: ProtocolKind::OpenAiResponses,
            base_url: base,
            api_key: SecretString::new(""),
            auth: Default::default(),
        };
        let headers = OpenAiResponsesAdapter::auth_headers(&no_key, false).unwrap();
        assert!(!headers.contains_key(AUTHORIZATION));
    }
}
