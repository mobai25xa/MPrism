//! Stable protocol errors.

use std::fmt;

/// High-level error classification for adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolErrorKind {
    InvalidConfiguration,
    InvalidRequest,
    Authentication,
    PermissionDenied,
    RateLimited,
    ProviderUnavailable,
    Timeout,
    Transport,
    Decode,
    UnexpectedEof,
    Unsupported,
}

impl ProtocolErrorKind {
    pub fn retryable(self) -> bool {
        matches!(
            self,
            Self::RateLimited | Self::ProviderUnavailable | Self::Timeout
        )
    }
}

/// Adapter error returned to the desktop application layer.
#[derive(Debug, Clone)]
pub struct ProtocolError {
    pub kind: ProtocolErrorKind,
    pub message: String,
    pub retryable: bool,
    pub http_status: Option<u16>,
    pub provider_code: Option<String>,
    pub request_id: Option<String>,
}

impl ProtocolError {
    pub fn new(kind: ProtocolErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            retryable: kind.retryable(),
            http_status: None,
            provider_code: None,
            request_id: None,
        }
    }

    pub fn with_http_status(mut self, status: u16) -> Self {
        self.http_status = Some(status);
        self
    }

    pub fn with_provider_code(mut self, code: impl Into<String>) -> Self {
        self.provider_code = Some(code.into());
        self
    }

    pub fn with_request_id(mut self, id: impl Into<String>) -> Self {
        self.request_id = Some(id.into());
        self
    }

    pub fn with_retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ProtocolError {}

/// Map HTTP status codes to protocol error kinds.
pub fn kind_from_status(status: u16) -> ProtocolErrorKind {
    match status {
        400 | 404 | 422 => ProtocolErrorKind::InvalidRequest,
        401 => ProtocolErrorKind::Authentication,
        403 => ProtocolErrorKind::PermissionDenied,
        408 => ProtocolErrorKind::Timeout,
        429 => ProtocolErrorKind::RateLimited,
        500..=599 => ProtocolErrorKind::ProviderUnavailable,
        _ => ProtocolErrorKind::Transport,
    }
}

/// Redact obvious secrets from provider error text.
pub fn redact_secrets(input: &str) -> String {
    let mut out = input.to_string();
    // Authorization header fragments
    if let Some(idx) = out.to_ascii_lowercase().find("bearer ") {
        let rest = &out[idx + "bearer ".len()..];
        let end = rest
            .find(|c: char| c.is_whitespace() || c == '"' || c == ',')
            .unwrap_or(rest.len());
        let token = &rest[..end];
        if !token.is_empty() {
            out = out.replacen(token, "[REDACTED]", 1);
        }
    }
    // sk- style keys
    let mut redacted = String::with_capacity(out.len());
    let bytes = out.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 3 <= bytes.len() && &out[i..i + 3] == "sk-" {
            redacted.push_str("sk-[REDACTED]");
            i += 3;
            while i < bytes.len() {
                let c = bytes[i] as char;
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    i += 1;
                } else {
                    break;
                }
            }
            continue;
        }
        redacted.push(bytes[i] as char);
        i += 1;
    }
    redacted
}

/// Extract message/code from common OpenAI error JSON.
pub fn parse_provider_error_body(body: &str) -> (Option<String>, Option<String>) {
    let value: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return (None, None),
    };
    let err = value.get("error");
    let message = err
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
        .map(redact_secrets)
        .or_else(|| {
            value
                .get("message")
                .and_then(|m| m.as_str())
                .map(redact_secrets)
        });
    let code = err.and_then(|e| e.get("code")).and_then(|c| {
        c.as_str()
            .map(|s| s.to_string())
            .or_else(|| c.as_i64().map(|n| n.to_string()))
    });
    (message, code)
}

pub const ERROR_BODY_LIMIT: usize = 4 * 1024;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_status_codes() {
        assert_eq!(kind_from_status(401), ProtocolErrorKind::Authentication);
        assert_eq!(kind_from_status(429), ProtocolErrorKind::RateLimited);
        assert!(kind_from_status(429).retryable());
        assert!(!kind_from_status(401).retryable());
        assert_eq!(
            kind_from_status(503),
            ProtocolErrorKind::ProviderUnavailable
        );
    }

    #[test]
    fn redacts_keys_in_messages() {
        let raw = r#"Incorrect API key provided: sk-abc123DEF. Authorization: Bearer sk-abc123DEF"#;
        let redacted = redact_secrets(raw);
        assert!(!redacted.contains("sk-abc123DEF"));
        assert!(redacted.contains("REDACTED"));
    }

    #[test]
    fn parses_error_json() {
        let body = r#"{"error":{"message":"bad sk-secret-key","type":"invalid_request_error","code":"invalid_api_key"}}"#;
        let (msg, code) = parse_provider_error_body(body);
        assert_eq!(code.as_deref(), Some("invalid_api_key"));
        assert!(msg.unwrap().contains("REDACTED"));
    }
}
