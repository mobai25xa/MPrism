//! Public protocol types for MPrism model adapters.

use crate::error::{ProtocolError, ProtocolErrorKind};
use crate::secret::SecretString;
use url::Url;

/// Supported wire protocols.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProtocolKind {
    /// OpenAI Chat Completions (`POST /chat/completions`).
    OpenAiChatCompletions,
    /// OpenAI Responses API (`POST /responses`).
    OpenAiResponses,
    /// Anthropic Messages API (`POST /messages`).
    AnthropicMessages,
    /// Gemini generateContent / streamGenerateContent.
    GeminiGenerateContent,
}

/// Provider connection settings used by adapters.
#[derive(Debug, Clone)]
pub struct ProviderEndpoint {
    pub protocol: ProtocolKind,
    pub base_url: Url,
    pub api_key: SecretString,
}

impl ProviderEndpoint {
    /// Build an endpoint after validating base URL shape.
    pub fn new(
        protocol: ProtocolKind,
        base_url: impl AsRef<str>,
        api_key: impl Into<SecretString>,
    ) -> Result<Self, ProtocolError> {
        let base_url = normalize_base_url(base_url.as_ref())?;
        Ok(Self {
            protocol,
            base_url,
            api_key: api_key.into(),
        })
    }
}

/// Normalize provider base URL: no query/fragment, strip trailing slash, keep path.
pub fn normalize_base_url(raw: &str) -> Result<Url, ProtocolError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ProtocolError::new(
            ProtocolErrorKind::InvalidConfiguration,
            "Base URL 不能为空",
        ));
    }

    let mut url = Url::parse(trimmed).map_err(|err| {
        ProtocolError::new(
            ProtocolErrorKind::InvalidConfiguration,
            format!("Base URL 无效: {err}"),
        )
    })?;

    match url.scheme() {
        "http" | "https" => {}
        other => {
            return Err(ProtocolError::new(
                ProtocolErrorKind::InvalidConfiguration,
                format!("Base URL 仅支持 http/https，收到: {other}"),
            ));
        }
    }

    if url.query().is_some() || url.fragment().is_some() {
        return Err(ProtocolError::new(
            ProtocolErrorKind::InvalidConfiguration,
            "Base URL 不能包含 query 或 fragment",
        ));
    }

    // Strip trailing slash from path without dropping useful segments like /v1.
    let path = url.path().to_string();
    if path.len() > 1 && path.ends_with('/') {
        url.set_path(path.trim_end_matches('/'));
    }

    Ok(url)
}

/// Join `segment` onto base path while preserving existing path segments.
pub fn join_api_path(base: &Url, segment: &str) -> Result<Url, ProtocolError> {
    let segment = segment.trim_matches('/');
    if segment.is_empty() {
        return Err(ProtocolError::new(
            ProtocolErrorKind::InvalidConfiguration,
            "API 路径段不能为空",
        ));
    }

    let mut url = base.clone();
    let mut path = url.path().trim_end_matches('/').to_string();
    if path.is_empty() {
        path = String::from("/");
    }
    if path == "/" {
        path = format!("/{segment}");
    } else {
        path = format!("{path}/{segment}");
    }
    url.set_path(&path);
    Ok(url)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelInfo {
    pub id: String,
    pub display_name: String,
    pub owned_by: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatRole {
    System,
    User,
    Assistant,
}

impl ChatRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
}

impl ChatRequest {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.model.trim().is_empty() {
            return Err(ProtocolError::new(
                ProtocolErrorKind::InvalidRequest,
                "model 不能为空",
            ));
        }
        if !self.messages.iter().any(|m| m.role == ChatRole::User) {
            return Err(ProtocolError::new(
                ProtocolErrorKind::InvalidRequest,
                "messages 至少需要一条 user 消息",
            ));
        }
        if let Some(temp) = self.temperature {
            if !(0.0..=2.0).contains(&temp) {
                return Err(ProtocolError::new(
                    ProtocolErrorKind::InvalidRequest,
                    "temperature 必须在 0.0..=2.0",
                ));
            }
        }
        if let Some(max_tokens) = self.max_tokens {
            if max_tokens == 0 {
                return Err(ProtocolError::new(
                    ProtocolErrorKind::InvalidRequest,
                    "max_tokens 必须大于 0",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TokenUsage {
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
    pub total_tokens: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEvent {
    ReasoningDelta { text: String },
    ContentDelta { text: String },
    Usage(TokenUsage),
    Completed { finish_reason: Option<String> },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_base_url_and_keeps_v1() {
        let url = normalize_base_url("https://api.example.com/v1/").unwrap();
        assert_eq!(url.as_str(), "https://api.example.com/v1");
        let models = join_api_path(&url, "models").unwrap();
        assert_eq!(models.as_str(), "https://api.example.com/v1/models");
        let chat = join_api_path(&url, "chat/completions").unwrap();
        assert_eq!(chat.as_str(), "https://api.example.com/v1/chat/completions");
    }

    #[test]
    fn rejects_query_and_fragment() {
        assert!(normalize_base_url("https://api.example.com/v1?x=1").is_err());
        assert!(normalize_base_url("https://api.example.com/v1#frag").is_err());
    }

    #[test]
    fn validates_chat_request() {
        let ok = ChatRequest {
            model: "gpt".into(),
            messages: vec![ChatMessage {
                role: ChatRole::User,
                content: "hi".into(),
            }],
            temperature: Some(1.0),
            max_tokens: Some(16),
        };
        assert!(ok.validate().is_ok());

        let bad = ChatRequest {
            model: "  ".into(),
            messages: vec![ChatMessage {
                role: ChatRole::System,
                content: "sys".into(),
            }],
            temperature: Some(3.0),
            max_tokens: Some(0),
        };
        assert!(bad.validate().is_err());
    }
}
