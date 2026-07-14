//! OpenAI-compatible Chat Completions adapter.

use super::chunk::{decode_sse_data, ChunkDecodeState};
use crate::adapter::{ChatStream, ProtocolAdapter};
use crate::error::{
    kind_from_status, parse_provider_error_body, redact_secrets, ProtocolError, ProtocolErrorKind,
    ERROR_BODY_LIMIT,
};
use crate::sse::SseParser;
use crate::types::{
    join_api_path, ChatRequest, ModelInfo, ProtocolKind, ProviderEndpoint, StreamEvent,
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
const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(120);

/// OpenAI-compatible adapter (Chat Completions + Models).
#[derive(Debug, Clone)]
pub struct OpenAiCompatibleAdapter {
    client: Client,
}

impl OpenAiCompatibleAdapter {
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
        headers
    }

    async fn map_error_response(response: Response) -> ProtocolError {
        let status = response.status();
        let request_id = response
            .headers()
            .get("x-request-id")
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
        let (message, code) = parse_provider_error_body(&body);
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
        if endpoint.protocol != ProtocolKind::OpenAiChatCompletions {
            return Err(ProtocolError::new(
                ProtocolErrorKind::Unsupported,
                "协议不受支持",
            ));
        }
        request.validate()?;

        let url = join_api_path(&endpoint.base_url, "chat/completions")?;
        let wire = WireChatRequest::from_request(&request);
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
            let mut decode_state = ChunkDecodeState::default();
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
                            if decode_state.finish_reason.is_some() {
                                yield Ok(StreamEvent::Completed {
                                    finish_reason: decode_state.finish_reason.clone(),
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
    messages: Vec<WireChatMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Debug, Serialize)]
struct WireChatMessage {
    role: String,
    content: String,
}

impl WireChatRequest {
    fn from_request(request: &ChatRequest) -> Self {
        Self {
            model: request.model.trim().to_string(),
            messages: request
                .messages
                .iter()
                .map(|m| WireChatMessage {
                    role: m.role.as_str().to_string(),
                    content: m.content.clone(),
                })
                .collect(),
            stream: true,
            temperature: request.temperature,
            max_tokens: request.max_tokens,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret::SecretString;
    use crate::types::normalize_base_url;
    use crate::types::ChatRole;

    #[test]
    fn wire_request_omits_optional_none() {
        let req = ChatRequest {
            model: "m".into(),
            messages: vec![crate::types::ChatMessage {
                role: ChatRole::User,
                content: "hi".into(),
            }],
            temperature: None,
            max_tokens: None,
        };
        let wire = WireChatRequest::from_request(&req);
        let json = serde_json::to_value(&wire).unwrap();
        assert_eq!(json["stream"], true);
        assert!(json.get("temperature").is_none());
        assert!(json.get("max_tokens").is_none());
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
        };
        let headers = OpenAiCompatibleAdapter::auth_headers(&with_key, true);
        assert!(headers.contains_key(AUTHORIZATION));

        let no_key = ProviderEndpoint {
            protocol: ProtocolKind::OpenAiChatCompletions,
            base_url: base,
            api_key: SecretString::new(""),
        };
        let headers = OpenAiCompatibleAdapter::auth_headers(&no_key, false);
        assert!(!headers.contains_key(AUTHORIZATION));
    }
}
