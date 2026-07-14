//! Anthropic Messages adapter (`POST /messages` + `GET /models`).

use super::sse_events::{decode_sse_data, EventDecodeState};
use crate::adapter::{ChatStream, ProtocolAdapter};
use crate::error::{
    kind_from_status, parse_provider_error_body, redact_secrets, ProtocolError, ProtocolErrorKind,
    ERROR_BODY_LIMIT,
};
use crate::sse::SseParser;
use crate::types::{
    join_api_path, ChatRequest, ChatRole, ModelInfo, ProtocolKind, ProviderEndpoint, StreamEvent,
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
const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(120);
/// Anthropic requires `max_tokens`; used when ChatRequest omits it.
const DEFAULT_MAX_TOKENS: u32 = 4096;
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Anthropic Messages adapter (SSE streaming).
#[derive(Debug, Clone)]
pub struct AnthropicMessagesAdapter {
    client: Client,
}

impl AnthropicMessagesAdapter {
    /// Create an adapter with default HTTP timeouts.
    pub fn new() -> Result<Self, ProtocolError> {
        let client = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .map_err(|err| {
                ProtocolError::new(
                    ProtocolErrorKind::Transport,
                    format!("创建 HTTP 客户端失败: {err}"),
                )
            })?;
        Ok(Self { client })
    }

    fn auth_headers(endpoint: &ProviderEndpoint, accept_event_stream: bool) -> HeaderMap {
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
        headers
    }

    async fn map_error_response(response: Response) -> ProtocolError {
        let status = response.status();
        let request_id = response
            .headers()
            .get("request-id")
            .or_else(|| response.headers().get("x-request-id"))
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let kind = kind_from_status(status.as_u16());
        let bytes = response.bytes().await.unwrap_or_default();
        let limited = if bytes.len() > ERROR_BODY_LIMIT {
            &bytes[..ERROR_BODY_LIMIT]
        } else {
            &bytes
        };
        let body = String::from_utf8_lossy(limited);
        let (message, code) = parse_anthropic_error_body(&body);
        let message = message.unwrap_or_else(|| {
            if body.trim().is_empty() {
                format!("服务商返回 HTTP {}", status.as_u16())
            } else {
                redact_secrets(body.trim())
            }
        });
        let mut err = ProtocolError::new(kind, message).with_http_status(status.as_u16());
        if let Some(code) = code {
            err = err.with_provider_code(code);
        }
        if let Some(id) = request_id {
            err = err.with_request_id(id);
        }
        err
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

        let url = join_api_path(&endpoint.base_url, "models")?;
        let request = self
            .client
            .get(url)
            .headers(Self::auth_headers(endpoint, false));

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

        let url = join_api_path(&endpoint.base_url, "messages")?;
        let wire = WireMessagesRequest::from_request(&request);
        let http_request = self
            .client
            .post(url)
            .headers(Self::auth_headers(endpoint, true))
            .json(&wire);

        let response = http_request.send().await.map_err(map_reqwest_error)?;
        if !response.status().is_success() {
            return Err(Self::map_error_response(response).await);
        }

        let mut byte_stream = response.bytes_stream();
        let stream = async_stream::stream! {
            let mut parser = SseParser::new();
            let mut decode_state = EventDecodeState::default();
            let mut completed = false;
            let mut failed = false;

            while !completed && !failed {
                let next = timeout(STREAM_IDLE_TIMEOUT, byte_stream.next()).await;
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

/// Parse Anthropic error JSON; prefer `error.type` as provider code.
fn parse_anthropic_error_body(body: &str) -> (Option<String>, Option<String>) {
    let (message, openai_code) = parse_provider_error_body(body);
    let value: Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return (message, openai_code),
    };
    let code = value
        .get("error")
        .and_then(|e| e.get("type"))
        .and_then(|t| t.as_str())
        .map(|s| s.to_string())
        .or(openai_code);
    (message, code)
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
    messages: Vec<WireChatMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Debug, Serialize)]
struct WireChatMessage {
    role: String,
    content: String,
}

impl WireMessagesRequest {
    fn from_request(request: &ChatRequest) -> Self {
        let mut system_parts = Vec::new();
        let mut messages = Vec::new();
        for m in &request.messages {
            match m.role {
                ChatRole::System => {
                    if !m.content.is_empty() {
                        system_parts.push(m.content.clone());
                    }
                }
                ChatRole::User | ChatRole::Assistant => {
                    messages.push(WireChatMessage {
                        role: m.role.as_str().to_string(),
                        content: m.content.clone(),
                    });
                }
            }
        }
        let system = if system_parts.is_empty() {
            None
        } else {
            Some(system_parts.join("\n\n"))
        };
        Self {
            model: request.model.trim().to_string(),
            max_tokens: request.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
            messages,
            stream: true,
            system,
            temperature: request.temperature,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret::SecretString;
    use crate::types::{normalize_base_url, ChatMessage};

    #[test]
    fn kind_is_anthropic_messages() {
        let adapter = AnthropicMessagesAdapter::new().unwrap();
        assert_eq!(adapter.kind(), ProtocolKind::AnthropicMessages);
    }

    #[test]
    fn wire_request_system_and_default_max_tokens() {
        let req = ChatRequest {
            model: "claude".into(),
            messages: vec![
                ChatMessage {
                    role: ChatRole::System,
                    content: "sys".into(),
                },
                ChatMessage {
                    role: ChatRole::User,
                    content: "hi".into(),
                },
            ],
            temperature: Some(0.5),
            max_tokens: None,
        };
        let wire = WireMessagesRequest::from_request(&req);
        let json = serde_json::to_value(&wire).unwrap();
        assert_eq!(json["stream"], true);
        assert_eq!(json["max_tokens"], 4096);
        assert_eq!(json["system"], "sys");
        assert_eq!(json["temperature"], 0.5);
        assert_eq!(json["messages"].as_array().unwrap().len(), 1);
        assert_eq!(json["messages"][0]["role"], "user");
        assert!(json.get("tools").is_none());
    }

    #[test]
    fn wire_request_uses_explicit_max_tokens() {
        let req = ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessage {
                role: ChatRole::User,
                content: "hi".into(),
            }],
            temperature: None,
            max_tokens: Some(128),
        };
        let wire = WireMessagesRequest::from_request(&req);
        let json = serde_json::to_value(&wire).unwrap();
        assert_eq!(json["max_tokens"], 128);
        assert!(json.get("system").is_none());
        assert!(json.get("temperature").is_none());
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
        };
        let headers = AnthropicMessagesAdapter::auth_headers(&with_key, true);
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
        };
        let headers = AnthropicMessagesAdapter::auth_headers(&no_key, false);
        assert!(!headers.contains_key("x-api-key"));
        assert!(headers.contains_key("anthropic-version"));
    }

    #[test]
    fn parse_anthropic_error_uses_type_as_code() {
        let body = r#"{"type":"error","error":{"type":"authentication_error","message":"bad sk-secret-key"}}"#;
        let (msg, code) = parse_anthropic_error_body(body);
        assert_eq!(code.as_deref(), Some("authentication_error"));
        assert!(msg.unwrap().contains("REDACTED"));
    }
}
