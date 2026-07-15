//! OpenAI-compatible Chat Completions adapter.

use super::chunk::{decode_sse_data, ChunkDecodeState};
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

/// OpenAI-compatible adapter (Chat Completions + Models).
#[derive(Debug, Clone)]
pub struct OpenAiCompatibleAdapter {
    client: Client,
    stream_idle_timeout: Duration,
}

impl OpenAiCompatibleAdapter {
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

impl Default for OpenAiCompatibleAdapter {
    fn default() -> Self {
        Self::new().expect("reqwest client")
    }
}

#[async_trait]
impl ProtocolAdapter for OpenAiCompatibleAdapter {
    fn kind(&self) -> ProtocolKind {
        ProtocolKind::OpenAiChatCompletions
    }

    fn capabilities(&self) -> ProtocolCapabilities {
        ProtocolCapabilities {
            streaming: true,
            list_models: true,
            reasoning_output: true,
            reasoning_control: false,
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
        if endpoint.protocol != ProtocolKind::OpenAiChatCompletions {
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
        if endpoint.protocol != ProtocolKind::OpenAiChatCompletions {
            return Err(ProtocolError::new(
                ProtocolErrorKind::Unsupported,
                "协议不受支持",
            ));
        }
        request.validate()?;
        request.check_capabilities(&self.capabilities())?;

        let url = apply_api_key_query(
            join_api_path(&endpoint.base_url, "chat/completions")?,
            endpoint,
        );
        let wire = WireChatRequest::try_from_request(&request)?;
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
            let mut decode_state = ChunkDecodeState::default();
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
                            if decode_state.finish_reason.is_some() {
                                yield Ok(StreamEvent::Completed {
                                    finish_reason: crate::finish::openai_chat_completions(
                                        decode_state.finish_reason.as_deref(),
                                    ),
                                });
                            } else {
                                yield Err(ProtocolError::new(
                                    ProtocolErrorKind::UnexpectedEof,
                                    "流在完成前意外结束",
                                ));
                            }
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
struct WireChatRequest {
    model: String,
    messages: Vec<Value>,
    stream: bool,
    /// Official field for streaming usage; not part of public ChatRequest API.
    stream_options: WireStreamOptions,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<WireTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<Value>,
}

#[derive(Debug, Serialize)]
struct WireStreamOptions {
    include_usage: bool,
}

#[derive(Debug, Serialize)]
struct WireTool {
    #[serde(rename = "type")]
    tool_type: &'static str,
    function: WireFunctionDef,
}

#[derive(Debug, Serialize)]
struct WireFunctionDef {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    parameters: Value,
}

impl WireChatRequest {
    /// Encode Chat Completions body. Reasoning control is unsupported (mapping-v2 §6).
    /// Tools are wire-only; this crate does not execute them.
    fn try_from_request(request: &ChatRequest) -> Result<Self, ProtocolError> {
        validate_chat_completions_reasoning(request.reasoning.as_ref())?;
        let tools = encode_chat_tools(request.tools.as_ref());
        let tool_choice = encode_chat_tool_choice(request.tool_choice.as_ref(), tools.is_some());
        Ok(Self {
            model: request.model.trim().to_string(),
            messages: request
                .messages
                .iter()
                .map(encode_chat_message)
                .collect::<Result<Vec<_>, _>>()?,
            stream: true,
            stream_options: WireStreamOptions {
                include_usage: true,
            },
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            tools,
            tool_choice,
        })
    }
}

fn encode_chat_tools(tools: Option<&Vec<crate::types::ToolDefinition>>) -> Option<Vec<WireTool>> {
    tools.map(|tools| {
        tools
            .iter()
            .map(|t| WireTool {
                tool_type: "function",
                function: WireFunctionDef {
                    name: t.name.trim().to_string(),
                    description: t.description.clone(),
                    parameters: t.parameters.clone(),
                },
            })
            .collect()
    })
}

fn encode_chat_tool_choice(
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
            "function": { "name": name }
        })),
    }
}

fn encode_chat_message(message: &crate::types::ChatMessage) -> Result<Value, ProtocolError> {
    use crate::types::ChatRole;
    match message.role {
        ChatRole::Tool => {
            let tool_call_id = message
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
            Ok(serde_json::json!({
                "role": "tool",
                "tool_call_id": tool_call_id,
                "content": message.text_content(),
            }))
        }
        ChatRole::Assistant if !message.tool_calls.is_empty() => {
            let tool_calls: Vec<Value> = message
                .tool_calls
                .iter()
                .map(|tc| {
                    serde_json::json!({
                        "id": tc.id,
                        "type": "function",
                        "function": {
                            "name": tc.name,
                            "arguments": tc.arguments,
                        }
                    })
                })
                .collect();
            let mut obj = serde_json::Map::new();
            obj.insert("role".into(), Value::String("assistant".into()));
            obj.insert("content".into(), encode_chat_completions_content(message)?);
            obj.insert("tool_calls".into(), Value::Array(tool_calls));
            Ok(Value::Object(obj))
        }
        role => Ok(serde_json::json!({
            "role": role.as_str(),
            "content": encode_chat_completions_content(message)?,
        })),
    }
}

/// mapping-v2 §4: single Text → string; multi-part or image → content array.
fn encode_chat_completions_content(
    message: &crate::types::ChatMessage,
) -> Result<Value, ProtocolError> {
    use crate::types::ContentPart;
    if !needs_multipart_content(&message.parts) {
        let text = message.text_content();
        // Assistant + tool_calls only may have empty content.
        if text.is_empty() && !message.tool_calls.is_empty() {
            return Ok(Value::Null);
        }
        return Ok(Value::String(text));
    }

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
            ContentPart::ImageUrl { url, detail } => {
                let mut image_url = serde_json::Map::new();
                image_url.insert("url".into(), Value::String(url.clone()));
                if let Some(d) = detail.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    image_url.insert("detail".into(), Value::String(d.to_string()));
                }
                blocks.push(serde_json::json!({
                    "type": "image_url",
                    "image_url": Value::Object(image_url),
                }));
            }
            ContentPart::ImageBase64 { media_type, data } => {
                let data_url = format_data_url(media_type, data);
                blocks.push(serde_json::json!({
                    "type": "image_url",
                    "image_url": { "url": data_url },
                }));
            }
        }
    }
    if blocks.is_empty() {
        if !message.tool_calls.is_empty() {
            return Ok(Value::Null);
        }
        return Err(ProtocolError::new(
            ProtocolErrorKind::InvalidRequest,
            "消息 content 编码结果为空",
        ));
    }
    Ok(Value::Array(blocks))
}

fn needs_multipart_content(parts: &[crate::types::ContentPart]) -> bool {
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
    // Multiple text parts → array per mapping-v2 §4.
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

/// Chat Completions: only None/Auto. On/Off/effort/budget → Unsupported.
fn validate_chat_completions_reasoning(
    policy: Option<&crate::types::ReasoningPolicy>,
) -> Result<(), ProtocolError> {
    use crate::types::ReasoningMode;
    let Some(policy) = policy else {
        return Ok(());
    };
    // Auto: ignore effort/budget; do not send control fields.
    if matches!(policy.mode, ReasoningMode::Auto) {
        return Ok(());
    }
    Err(ProtocolError::new(
        ProtocolErrorKind::Unsupported,
        "OpenAI Chat Completions 不支持推理控制（请使用 Responses 或 None/Auto）",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret::SecretString;
    use crate::types::normalize_base_url;
    use crate::types::{ChatRole, ReasoningEffort, ReasoningMode, ReasoningPolicy};

    #[test]
    fn wire_request_omits_optional_none() {
        let req = ChatRequest {
            model: "m".into(),
            messages: vec![crate::types::ChatMessage::text(ChatRole::User, "hi")],
            temperature: None,
            max_tokens: None,
            reasoning: None,
            tools: None,
            tool_choice: None,
        };
        let wire = WireChatRequest::try_from_request(&req).unwrap();
        let json = serde_json::to_value(&wire).unwrap();
        assert_eq!(json["stream"], true);
        assert_eq!(json["stream_options"]["include_usage"], true);
        assert!(json.get("temperature").is_none());
        assert!(json.get("max_tokens").is_none());
        assert!(json.get("reasoning").is_none());
        assert!(json.get("reasoning_effort").is_none());
    }

    #[test]
    fn reasoning_auto_ok_on_and_off_unsupported() {
        let auto = ChatRequest {
            model: "m".into(),
            messages: vec![crate::types::ChatMessage::text(ChatRole::User, "hi")],
            temperature: None,
            max_tokens: None,
            reasoning: Some(ReasoningPolicy {
                mode: ReasoningMode::Auto,
                effort: Some(ReasoningEffort::High),
                budget_tokens: Some(1000),
            }),
            tools: None,
            tool_choice: None,
        };
        let wire = WireChatRequest::try_from_request(&auto).unwrap();
        let json = serde_json::to_value(&wire).unwrap();
        assert!(json.get("reasoning").is_none());

        for mode in [ReasoningMode::On, ReasoningMode::Off] {
            let req = ChatRequest {
                model: "m".into(),
                messages: vec![crate::types::ChatMessage::text(ChatRole::User, "hi")],
                temperature: None,
                max_tokens: None,
                reasoning: Some(ReasoningPolicy {
                    mode,
                    effort: Some(ReasoningEffort::Low),
                    budget_tokens: None,
                }),
                tools: None,
                tool_choice: None,
            };
            let err = WireChatRequest::try_from_request(&req).unwrap_err();
            assert_eq!(err.kind, ProtocolErrorKind::Unsupported);
        }
    }

    #[test]
    fn capabilities_match_matrix_v2() {
        let caps = OpenAiCompatibleAdapter::new().unwrap().capabilities();
        assert!(caps.streaming);
        assert!(caps.list_models);
        assert!(caps.reasoning_output);
        assert!(!caps.reasoning_control);
        assert!(caps.tools);
        assert!(caps.vision_input);
        assert!(caps.stream_usage);
        assert!(caps.custom_headers);
        assert!(caps.api_key_query);
    }

    #[test]
    fn tools_wire_encoding_and_none_omitted() {
        use crate::types::{ToolChoice, ToolDefinition};
        let none = ChatRequest {
            model: "m".into(),
            messages: vec![crate::types::ChatMessage::text(ChatRole::User, "hi")],
            temperature: None,
            max_tokens: None,
            reasoning: None,
            tools: None,
            tool_choice: None,
        };
        let json = serde_json::to_value(WireChatRequest::try_from_request(&none).unwrap()).unwrap();
        assert!(json.get("tools").is_none());
        assert!(json.get("tool_choice").is_none());

        let with = ChatRequest {
            model: "m".into(),
            messages: vec![
                crate::types::ChatMessage::text(ChatRole::User, "hi"),
                crate::types::ChatMessage {
                    role: ChatRole::Assistant,
                    parts: vec![],
                    tool_call_id: None,
                    tool_calls: vec![crate::types::ToolCall {
                        id: "call_1".into(),
                        name: "lookup".into(),
                        arguments: r#"{"q":1}"#.into(),
                    }],
                },
                crate::types::ChatMessage {
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
                description: Some("d".into()),
                parameters: serde_json::json!({"type":"object"}),
            }]),
            tool_choice: Some(ToolChoice::Named {
                name: "lookup".into(),
            }),
        };
        assert!(with.validate().is_ok());
        let json = serde_json::to_value(WireChatRequest::try_from_request(&with).unwrap()).unwrap();
        assert_eq!(json["tools"][0]["type"], "function");
        assert_eq!(json["tools"][0]["function"]["name"], "lookup");
        assert_eq!(json["tool_choice"]["type"], "function");
        assert_eq!(json["tool_choice"]["function"]["name"], "lookup");
        assert_eq!(json["messages"][2]["role"], "tool");
        assert_eq!(json["messages"][2]["tool_call_id"], "call_1");
        assert_eq!(json["messages"][1]["tool_calls"][0]["id"], "call_1");

        for choice in [ToolChoice::Auto, ToolChoice::None, ToolChoice::Required] {
            let req = ChatRequest {
                model: "m".into(),
                messages: vec![crate::types::ChatMessage::text(ChatRole::User, "hi")],
                temperature: None,
                max_tokens: None,
                reasoning: None,
                tools: Some(vec![ToolDefinition {
                    name: "t".into(),
                    description: None,
                    parameters: serde_json::json!({}),
                }]),
                tool_choice: Some(choice.clone()),
            };
            let json =
                serde_json::to_value(WireChatRequest::try_from_request(&req).unwrap()).unwrap();
            match choice {
                ToolChoice::Auto => assert_eq!(json["tool_choice"], "auto"),
                ToolChoice::None => assert_eq!(json["tool_choice"], "none"),
                ToolChoice::Required => assert_eq!(json["tool_choice"], "required"),
                ToolChoice::Named { .. } => unreachable!(),
            }
        }
    }

    #[test]
    fn multimodal_wire_url_and_base64_and_text_only() {
        use crate::types::{ChatMessage, ContentPart};

        // Pure text remains string-shaped (V1 path).
        let text_only = ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessage::text(ChatRole::User, "hi")],
            temperature: None,
            max_tokens: None,
            reasoning: None,
            tools: None,
            tool_choice: None,
        };
        let json =
            serde_json::to_value(WireChatRequest::try_from_request(&text_only).unwrap()).unwrap();
        assert_eq!(json["messages"][0]["content"], "hi");

        let with_url = ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessage {
                role: ChatRole::User,
                parts: vec![
                    ContentPart::Text { text: "see".into() },
                    ContentPart::ImageUrl {
                        url: "https://example.com/a.png".into(),
                        detail: Some("high".into()),
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
            serde_json::to_value(WireChatRequest::try_from_request(&with_url).unwrap()).unwrap();
        let content = &json["messages"][0]["content"];
        assert!(content.is_array());
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "see");
        assert_eq!(content[1]["type"], "image_url");
        assert_eq!(content[1]["image_url"]["url"], "https://example.com/a.png");
        assert_eq!(content[1]["image_url"]["detail"], "high");

        let with_b64 = ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessage {
                role: ChatRole::User,
                parts: vec![
                    ContentPart::Text { text: "see".into() },
                    ContentPart::ImageBase64 {
                        media_type: "image/png".into(),
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
            serde_json::to_value(WireChatRequest::try_from_request(&with_b64).unwrap()).unwrap();
        assert_eq!(
            json["messages"][0]["content"][1]["image_url"]["url"],
            "data:image/png;base64,aGVsbG8="
        );
        assert!(
            OpenAiCompatibleAdapter::new()
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
        assert_eq!(models[0].owned_by.as_deref(), Some("x"));
        assert_eq!(models[1].id, "b");
    }

    #[test]
    fn auth_header_only_when_key_present() {
        let base = normalize_base_url("http://127.0.0.1:9/v1").unwrap();
        let with_key = ProviderEndpoint {
            protocol: ProtocolKind::OpenAiChatCompletions,
            base_url: base.clone(),
            api_key: SecretString::new("sk-test"),
            auth: Default::default(),
        };
        let headers = OpenAiCompatibleAdapter::auth_headers(&with_key, true).unwrap();
        assert!(headers.contains_key(AUTHORIZATION));

        let no_key = ProviderEndpoint {
            protocol: ProtocolKind::OpenAiChatCompletions,
            base_url: base,
            api_key: SecretString::new(""),
            auth: Default::default(),
        };
        let headers = OpenAiCompatibleAdapter::auth_headers(&no_key, false).unwrap();
        assert!(!headers.contains_key(AUTHORIZATION));
    }
}
