//! Gemini generateContent adapter (`streamGenerateContent` + `GET /models`).

use super::stream_decode::{decode_sse_data, EventDecodeState};
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
use url::Url;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const LIST_TIMEOUT: Duration = Duration::from_secs(30);
/// Stream idle timeout (no body bytes): model-protocol-sdk §5.
pub const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(120);

/// Gemini generateContent / streamGenerateContent adapter.
#[derive(Debug, Clone)]
pub struct GeminiGenerateContentAdapter {
    client: Client,
    stream_idle_timeout: Duration,
}

impl GeminiGenerateContentAdapter {
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
            if let Ok(v) = HeaderValue::from_str(endpoint.api_key.expose_secret()) {
                headers.insert(HeaderName::from_static("x-goog-api-key"), v);
            }
        }
        merge_extra_headers(&mut headers, &endpoint.auth)?;
        Ok(headers)
    }

    async fn map_error_response(response: Response) -> ProtocolError {
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let bytes = response.bytes().await.unwrap_or_default();
        map_http_error(status, &headers, &bytes, ErrorBodyFamily::Gemini)
    }
}

impl Default for GeminiGenerateContentAdapter {
    fn default() -> Self {
        Self::new().expect("reqwest client")
    }
}

#[async_trait]
impl ProtocolAdapter for GeminiGenerateContentAdapter {
    fn kind(&self) -> ProtocolKind {
        ProtocolKind::GeminiGenerateContent
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
        if endpoint.protocol != ProtocolKind::GeminiGenerateContent {
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
        if endpoint.protocol != ProtocolKind::GeminiGenerateContent {
            return Err(ProtocolError::new(
                ProtocolErrorKind::Unsupported,
                "协议不受支持",
            ));
        }
        request.validate()?;
        request.check_capabilities(&self.capabilities())?;

        let url = apply_api_key_query(
            stream_generate_content_url(&endpoint.base_url, &request.model)?,
            endpoint,
        );
        let wire = WireGenerateContentRequest::try_from_request(&request)?;
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
    let models = value
        .get("models")
        .and_then(|d| d.as_array())
        .ok_or_else(|| ProtocolError::new(ProtocolErrorKind::Decode, "模型列表缺少 models 数组"))?;

    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for item in models {
        let name = item
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let Some(name) = name else {
            continue;
        };
        let id = strip_models_prefix(name);
        if id.is_empty() || !seen.insert(id.clone()) {
            continue;
        }
        let display_name = item
            .get("displayName")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(id.as_str())
            .to_string();
        out.push(ModelInfo {
            id,
            display_name,
            owned_by: None,
        });
    }
    Ok(out)
}

fn strip_models_prefix(name: &str) -> String {
    let trimmed = name.trim();
    trimmed
        .strip_prefix("models/")
        .unwrap_or(trimmed)
        .to_string()
}

fn model_resource_path(model: &str) -> Result<String, ProtocolError> {
    let model = model.trim();
    if model.is_empty() {
        return Err(ProtocolError::new(
            ProtocolErrorKind::InvalidRequest,
            "model 不能为空",
        ));
    }
    if model.starts_with("models/") {
        Ok(format!("{model}:streamGenerateContent"))
    } else {
        Ok(format!("models/{model}:streamGenerateContent"))
    }
}

fn stream_generate_content_url(base: &Url, model: &str) -> Result<Url, ProtocolError> {
    let path_segment = model_resource_path(model)?;
    let mut url = join_api_path(base, &path_segment)?;
    url.query_pairs_mut().append_pair("alt", "sse");
    Ok(url)
}

#[derive(Debug, Serialize)]
struct WireGenerateContentRequest {
    contents: Vec<Value>,
    #[serde(rename = "systemInstruction", skip_serializing_if = "Option::is_none")]
    system_instruction: Option<Value>,
    #[serde(rename = "generationConfig", skip_serializing_if = "Option::is_none")]
    generation_config: Option<WireGenerationConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<Value>>,
    #[serde(rename = "toolConfig", skip_serializing_if = "Option::is_none")]
    tool_config: Option<Value>,
}

#[derive(Debug, Serialize)]
struct WireGenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(rename = "maxOutputTokens", skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    #[serde(rename = "thinkingConfig", skip_serializing_if = "Option::is_none")]
    thinking_config: Option<WireThinkingConfig>,
}

#[derive(Debug, Serialize)]
struct WireThinkingConfig {
    #[serde(rename = "includeThoughts", skip_serializing_if = "Option::is_none")]
    include_thoughts: Option<bool>,
    #[serde(rename = "thinkingBudget", skip_serializing_if = "Option::is_none")]
    thinking_budget: Option<u32>,
    #[serde(rename = "thinkingLevel", skip_serializing_if = "Option::is_none")]
    thinking_level: Option<String>,
}

impl WireGenerateContentRequest {
    fn try_from_request(request: &ChatRequest) -> Result<Self, ProtocolError> {
        let mut system_parts = Vec::new();
        let mut contents = Vec::new();
        for m in &request.messages {
            match m.role {
                ChatRole::System => {
                    if !m.text_content().is_empty() {
                        system_parts.push(m.text_content());
                    }
                }
                ChatRole::User => {
                    contents.push(serde_json::json!({
                        "role": "user",
                        "parts": encode_gemini_parts(m)?,
                    }));
                }
                ChatRole::Assistant => {
                    let mut parts = encode_gemini_parts(m)?;
                    for tc in &m.tool_calls {
                        let args = serde_json::from_str::<Value>(&tc.arguments)
                            .unwrap_or_else(|_| Value::String(tc.arguments.clone()));
                        // Preserve call id in name-only public model; Gemini uses name+args.
                        // If arguments already include thought signature fields from history,
                        // callers must pass them through `arguments` JSON as returned by the model.
                        parts.push(serde_json::json!({
                            "functionCall": {
                                "name": tc.name,
                                "args": args,
                            }
                        }));
                    }
                    if parts.is_empty() {
                        return Err(ProtocolError::new(
                            ProtocolErrorKind::InvalidRequest,
                            "消息 content 编码结果为空",
                        ));
                    }
                    contents.push(serde_json::json!({
                        "role": "model",
                        "parts": parts,
                    }));
                }
                ChatRole::Tool => {
                    let name = m
                        .tool_call_id
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .unwrap_or("tool");
                    // Public tool_call_id maps to function name for Gemini functionResponse
                    // when the app stores the function name in tool_call_id or a synthetic id.
                    // Prefer name from tool_call_id; response body is text parts.
                    let response = serde_json::from_str::<Value>(&m.text_content())
                        .unwrap_or_else(|_| serde_json::json!({ "result": m.text_content() }));
                    contents.push(serde_json::json!({
                        "role": "user",
                        "parts": [{
                            "functionResponse": {
                                "name": name,
                                "response": response,
                            }
                        }],
                    }));
                }
            }
        }
        let system_instruction = if system_parts.is_empty() {
            None
        } else {
            Some(serde_json::json!({
                "parts": [{ "text": system_parts.join("\n\n") }],
            }))
        };
        let thinking_config = encode_gemini_thinking(request.reasoning.as_ref(), &request.model)?;
        let generation_config = match (request.temperature, request.max_tokens, thinking_config) {
            (None, None, None) => None,
            (temperature, max_output_tokens, thinking_config) => Some(WireGenerationConfig {
                temperature,
                max_output_tokens,
                thinking_config,
            }),
        };
        let tools = encode_gemini_tools(request.tools.as_ref());
        let tool_config = encode_gemini_tool_config(request.tool_choice.as_ref(), tools.is_some());
        Ok(Self {
            contents,
            system_instruction,
            generation_config,
            tools,
            tool_config,
        })
    }
}

/// mapping-v2 §6: Text + ImageBase64; ImageUrl → Unsupported.
fn encode_gemini_parts(message: &crate::types::ChatMessage) -> Result<Vec<Value>, ProtocolError> {
    use crate::types::ContentPart;
    let mut parts = Vec::with_capacity(message.parts.len());
    for part in &message.parts {
        match part {
            ContentPart::Text { text } => {
                if text.is_empty() {
                    continue;
                }
                parts.push(serde_json::json!({ "text": text }));
            }
            ContentPart::ImageBase64 { media_type, data } => {
                let data: String = data.chars().filter(|c| !c.is_whitespace()).collect();
                parts.push(serde_json::json!({
                    "inline_data": {
                        "mime_type": media_type.trim(),
                        "data": data,
                    }
                }));
            }
            ContentPart::ImageUrl { .. } => {
                return Err(ProtocolError::new(
                    ProtocolErrorKind::Unsupported,
                    "Gemini V2 不映射 ImageUrl（任意 http(s) URL）；请使用 ImageBase64",
                ));
            }
        }
    }
    Ok(parts)
}

fn encode_gemini_tools(tools: Option<&Vec<crate::types::ToolDefinition>>) -> Option<Vec<Value>> {
    tools.map(|tools| {
        let decls: Vec<Value> = tools
            .iter()
            .map(|t| {
                let mut obj = serde_json::Map::new();
                obj.insert("name".into(), Value::String(t.name.trim().to_string()));
                if let Some(desc) = &t.description {
                    obj.insert("description".into(), Value::String(desc.clone()));
                }
                obj.insert("parameters".into(), t.parameters.clone());
                Value::Object(obj)
            })
            .collect();
        vec![serde_json::json!({
            "functionDeclarations": decls,
        })]
    })
}

fn encode_gemini_tool_config(
    choice: Option<&crate::types::ToolChoice>,
    has_tools: bool,
) -> Option<Value> {
    use crate::types::ToolChoice;
    let mode = match choice {
        None if has_tools => "AUTO",
        None => return None,
        Some(ToolChoice::Auto) => "AUTO",
        Some(ToolChoice::None) => "NONE",
        Some(ToolChoice::Required) => "ANY",
        Some(ToolChoice::Named { name }) => {
            return Some(serde_json::json!({
                "functionCallingConfig": {
                    "mode": "ANY",
                    "allowedFunctionNames": [name],
                }
            }));
        }
    };
    Some(serde_json::json!({
        "functionCallingConfig": { "mode": mode }
    }))
}

/// Prefer thinkingLevel (Gemini 3); budget-only models (2.5) use thinkingBudget.
fn model_prefers_budget_only(model: &str) -> bool {
    let id = model.trim().to_ascii_lowercase();
    let id = id.strip_prefix("models/").unwrap_or(&id);
    // 2.5 series documents thinkingBudget; treat as budget-only path.
    id.contains("2.5") || id.contains("2-5")
}

/// Encode `generationConfig.thinkingConfig` per mapping-v2 §4.
fn encode_gemini_thinking(
    policy: Option<&crate::types::ReasoningPolicy>,
    model: &str,
) -> Result<Option<WireThinkingConfig>, ProtocolError> {
    use crate::types::{ReasoningEffort, ReasoningMode};
    let Some(policy) = policy else {
        return Ok(None);
    };
    match policy.mode {
        ReasoningMode::Auto => Ok(None),
        ReasoningMode::Off => Ok(Some(WireThinkingConfig {
            include_thoughts: None,
            thinking_budget: Some(0),
            thinking_level: None,
        })),
        ReasoningMode::On => {
            // budget_tokens == 0 is treated as Off.
            if policy.budget_tokens == Some(0) {
                return Ok(Some(WireThinkingConfig {
                    include_thoughts: None,
                    thinking_budget: Some(0),
                    thinking_level: None,
                }));
            }
            // budget takes precedence over effort.
            if let Some(n) = policy.budget_tokens {
                return Ok(Some(WireThinkingConfig {
                    include_thoughts: Some(true),
                    thinking_budget: Some(n),
                    thinking_level: None,
                }));
            }
            let budget_only = model_prefers_budget_only(model);
            match policy.effort {
                None => {
                    if budget_only {
                        Ok(Some(WireThinkingConfig {
                            include_thoughts: Some(true),
                            thinking_budget: Some(4096),
                            thinking_level: None,
                        }))
                    } else {
                        Ok(Some(WireThinkingConfig {
                            include_thoughts: Some(true),
                            thinking_budget: None,
                            thinking_level: Some("medium".into()),
                        }))
                    }
                }
                Some(effort) => {
                    if budget_only {
                        let budget = match effort {
                            ReasoningEffort::Minimal => 1024,
                            ReasoningEffort::Low => 1024,
                            ReasoningEffort::Medium => 4096,
                            ReasoningEffort::High => 8192,
                            ReasoningEffort::XHigh | ReasoningEffort::Max => 8192,
                        };
                        Ok(Some(WireThinkingConfig {
                            include_thoughts: Some(true),
                            thinking_budget: Some(budget),
                            thinking_level: None,
                        }))
                    } else {
                        let level = match effort {
                            ReasoningEffort::Minimal => "minimal",
                            ReasoningEffort::Low => "low",
                            ReasoningEffort::Medium => "medium",
                            ReasoningEffort::High
                            | ReasoningEffort::XHigh
                            | ReasoningEffort::Max => "high",
                        };
                        Ok(Some(WireThinkingConfig {
                            include_thoughts: Some(true),
                            thinking_budget: None,
                            thinking_level: Some(level.into()),
                        }))
                    }
                }
            }
        }
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
    fn kind_is_gemini() {
        let adapter = GeminiGenerateContentAdapter::new().unwrap();
        assert_eq!(adapter.kind(), ProtocolKind::GeminiGenerateContent);
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
    fn tools_function_declarations_and_response() {
        use crate::types::{ToolChoice, ToolDefinition};
        let req = ChatRequest {
            model: "gemini-2.0-flash".into(),
            messages: vec![
                ChatMessage::text(ChatRole::User, "hi"),
                ChatMessage {
                    role: ChatRole::Assistant,
                    parts: vec![],
                    tool_call_id: None,
                    tool_calls: vec![crate::types::ToolCall {
                        id: "lookup".into(),
                        name: "lookup".into(),
                        arguments: r#"{"q":1}"#.into(),
                    }],
                },
                ChatMessage {
                    role: ChatRole::Tool,
                    parts: vec![crate::types::ContentPart::Text {
                        text: r#"{"result":"ok"}"#.into(),
                    }],
                    tool_call_id: Some("lookup".into()),
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
        let json =
            serde_json::to_value(WireGenerateContentRequest::try_from_request(&req).unwrap())
                .unwrap();
        assert_eq!(
            json["tools"][0]["functionDeclarations"][0]["name"],
            "lookup"
        );
        assert_eq!(json["toolConfig"]["functionCallingConfig"]["mode"], "ANY");
        assert_eq!(
            json["toolConfig"]["functionCallingConfig"]["allowedFunctionNames"][0],
            "lookup"
        );
        assert_eq!(
            json["contents"][1]["parts"][0]["functionCall"]["name"],
            "lookup"
        );
        assert_eq!(
            json["contents"][2]["parts"][0]["functionResponse"]["name"],
            "lookup"
        );
    }

    #[test]
    fn wire_request_system_and_roles() {
        let req = ChatRequest {
            model: "gemini-2.0-flash".into(),
            messages: vec![
                ChatMessage::text(ChatRole::System, "sys"),
                ChatMessage::text(ChatRole::User, "hi"),
                ChatMessage::text(ChatRole::Assistant, "yo"),
            ],
            temperature: Some(0.5),
            max_tokens: Some(128),
            reasoning: None,
            tools: None,
            tool_choice: None,
        };
        let wire = WireGenerateContentRequest::try_from_request(&req).unwrap();
        let json = serde_json::to_value(&wire).unwrap();
        assert_eq!(json["systemInstruction"]["parts"][0]["text"], "sys");
        assert_eq!(json["contents"][0]["role"], "user");
        assert_eq!(json["contents"][1]["role"], "model");
        assert_eq!(json["generationConfig"]["temperature"], 0.5);
        assert_eq!(json["generationConfig"]["maxOutputTokens"], 128);
        assert!(json.get("tools").is_none());
        assert!(json["generationConfig"].get("thinkingConfig").is_none());
    }

    #[test]
    fn wire_request_omits_generation_config_when_empty() {
        let req = ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessage::text(ChatRole::User, "hi")],
            temperature: None,
            max_tokens: None,
            reasoning: None,
            tools: None,
            tool_choice: None,
        };
        let wire = WireGenerateContentRequest::try_from_request(&req).unwrap();
        let json = serde_json::to_value(&wire).unwrap();
        assert!(json.get("generationConfig").is_none());
        assert!(json.get("systemInstruction").is_none());
    }

    #[test]
    fn reasoning_none_auto_off_on_level_and_budget() {
        let base = |model: &str, policy: Option<ReasoningPolicy>| ChatRequest {
            model: model.into(),
            messages: vec![ChatMessage::text(ChatRole::User, "hi")],
            temperature: None,
            max_tokens: None,
            reasoning: policy,
            tools: None,
            tool_choice: None,
        };

        let none_json = serde_json::to_value(
            WireGenerateContentRequest::try_from_request(&base("gemini-3-pro", None)).unwrap(),
        )
        .unwrap();
        assert!(none_json.get("generationConfig").is_none());

        let auto_json = serde_json::to_value(
            WireGenerateContentRequest::try_from_request(&base(
                "gemini-3-pro",
                Some(ReasoningPolicy {
                    mode: ReasoningMode::Auto,
                    effort: Some(ReasoningEffort::High),
                    budget_tokens: Some(1000),
                }),
            ))
            .unwrap(),
        )
        .unwrap();
        assert!(auto_json.get("generationConfig").is_none());

        let off_json = serde_json::to_value(
            WireGenerateContentRequest::try_from_request(&base(
                "gemini-3-pro",
                Some(ReasoningPolicy {
                    mode: ReasoningMode::Off,
                    effort: None,
                    budget_tokens: None,
                }),
            ))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            off_json["generationConfig"]["thinkingConfig"]["thinkingBudget"],
            0
        );

        // On + none → thinkingLevel medium (non-2.5)
        let on_def = serde_json::to_value(
            WireGenerateContentRequest::try_from_request(&base(
                "gemini-3-pro",
                Some(ReasoningPolicy {
                    mode: ReasoningMode::On,
                    effort: None,
                    budget_tokens: None,
                }),
            ))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            on_def["generationConfig"]["thinkingConfig"]["thinkingLevel"],
            "medium"
        );
        assert_eq!(
            on_def["generationConfig"]["thinkingConfig"]["includeThoughts"],
            true
        );

        // On + effort high → level high
        let on_high = serde_json::to_value(
            WireGenerateContentRequest::try_from_request(&base(
                "gemini-3-pro",
                Some(ReasoningPolicy {
                    mode: ReasoningMode::On,
                    effort: Some(ReasoningEffort::High),
                    budget_tokens: None,
                }),
            ))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            on_high["generationConfig"]["thinkingConfig"]["thinkingLevel"],
            "high"
        );

        // budget path for 2.5 model On+none → budget 4096
        let budget_def = serde_json::to_value(
            WireGenerateContentRequest::try_from_request(&base(
                "gemini-2.5-pro",
                Some(ReasoningPolicy {
                    mode: ReasoningMode::On,
                    effort: None,
                    budget_tokens: None,
                }),
            ))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            budget_def["generationConfig"]["thinkingConfig"]["thinkingBudget"],
            4096
        );

        // budget takes precedence
        let budget_wins = serde_json::to_value(
            WireGenerateContentRequest::try_from_request(&base(
                "gemini-3-pro",
                Some(ReasoningPolicy {
                    mode: ReasoningMode::On,
                    effort: Some(ReasoningEffort::High),
                    budget_tokens: Some(2000),
                }),
            ))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            budget_wins["generationConfig"]["thinkingConfig"]["thinkingBudget"],
            2000
        );
        assert!(budget_wins["generationConfig"]["thinkingConfig"]
            .get("thinkingLevel")
            .is_none());
    }

    #[test]
    fn multimodal_inline_data_and_image_url_unsupported() {
        use crate::types::{ChatMessage, ContentPart};

        let text_only = ChatRequest {
            model: "gemini-2.0-flash".into(),
            messages: vec![ChatMessage::text(ChatRole::User, "hi")],
            temperature: None,
            max_tokens: None,
            reasoning: None,
            tools: None,
            tool_choice: None,
        };
        let json =
            serde_json::to_value(WireGenerateContentRequest::try_from_request(&text_only).unwrap())
                .unwrap();
        assert_eq!(json["contents"][0]["parts"][0]["text"], "hi");

        let with_b64 = ChatRequest {
            model: "gemini-2.0-flash".into(),
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
            serde_json::to_value(WireGenerateContentRequest::try_from_request(&with_b64).unwrap())
                .unwrap();
        assert_eq!(json["contents"][0]["parts"][0]["text"], "see");
        assert_eq!(
            json["contents"][0]["parts"][1]["inline_data"]["mime_type"],
            "image/png"
        );
        assert_eq!(
            json["contents"][0]["parts"][1]["inline_data"]["data"],
            "aGVsbG8="
        );

        let with_url = ChatRequest {
            model: "gemini-2.0-flash".into(),
            messages: vec![ChatMessage {
                role: ChatRole::User,
                parts: vec![
                    ContentPart::Text { text: "see".into() },
                    ContentPart::ImageUrl {
                        url: "https://example.com/a.png".into(),
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
        let err = WireGenerateContentRequest::try_from_request(&with_url).unwrap_err();
        assert_eq!(err.kind, ProtocolErrorKind::Unsupported);
        assert!(
            GeminiGenerateContentAdapter::new()
                .unwrap()
                .capabilities()
                .vision_input
        );
    }

    #[test]
    fn parse_models_strips_prefix() {
        let body = br#"{
            "models":[
                {"name":"models/gemini-2.0-flash","displayName":"Flash"},
                {"name":"models/gemini-2.0-flash"},
                {"name":"  "},
                {"name":"models/gemini-pro"}
            ]
        }"#;
        let models = parse_models_body(body).unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "gemini-2.0-flash");
        assert_eq!(models[0].display_name, "Flash");
        assert_eq!(models[1].id, "gemini-pro");
        assert_eq!(models[1].display_name, "gemini-pro");
    }

    #[test]
    fn stream_url_adds_alt_sse() {
        let base = normalize_base_url("https://generativelanguage.googleapis.com/v1beta").unwrap();
        let url = stream_generate_content_url(&base, "gemini-2.0-flash").unwrap();
        assert_eq!(
            url.as_str(),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:streamGenerateContent?alt=sse"
        );
        let url2 = stream_generate_content_url(&base, "models/gemini-pro").unwrap();
        assert!(url2
            .path()
            .ends_with("/models/gemini-pro:streamGenerateContent"));
        assert_eq!(url2.query(), Some("alt=sse"));
    }

    #[test]
    fn auth_header_x_goog_api_key() {
        let base = normalize_base_url("http://127.0.0.1:9/v1beta").unwrap();
        let with_key = ProviderEndpoint {
            protocol: ProtocolKind::GeminiGenerateContent,
            base_url: base.clone(),
            api_key: SecretString::new("AIza-test"),
            auth: Default::default(),
        };
        let headers = GeminiGenerateContentAdapter::auth_headers(&with_key, true).unwrap();
        assert_eq!(
            headers.get("x-goog-api-key").and_then(|v| v.to_str().ok()),
            Some("AIza-test")
        );
        assert!(!headers.contains_key(reqwest::header::AUTHORIZATION));

        let no_key = ProviderEndpoint {
            protocol: ProtocolKind::GeminiGenerateContent,
            base_url: base,
            api_key: SecretString::new(""),
            auth: Default::default(),
        };
        let headers = GeminiGenerateContentAdapter::auth_headers(&no_key, false).unwrap();
        assert!(!headers.contains_key("x-goog-api-key"));
    }

    #[test]
    fn parse_gemini_error_uses_status() {
        let body =
            r#"{"error":{"code":401,"message":"bad key AIza-secret","status":"UNAUTHENTICATED"}}"#;
        let (msg, code) = crate::error::parse_gemini_error_body(body);
        assert_eq!(code.as_deref(), Some("UNAUTHENTICATED"));
        assert!(msg.is_some());
    }
}
