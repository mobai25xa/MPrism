//! Stable protocol errors.
//!
//! Semantics follow `docs/sources/model-protocol-sdk.md` §4.

use reqwest::header::{HeaderMap, RETRY_AFTER};
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

/// High-level error classification for adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolErrorKind {
    InvalidConfiguration,
    InvalidRequest,
    Authentication,
    PermissionDenied,
    RateLimited,
    ContextLengthExceeded,
    ContentFilter,
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

/// Vendor error JSON family for body parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorBodyFamily {
    OpenAi,
    Anthropic,
    Gemini,
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
    pub retry_after_ms: Option<u64>,
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
            retry_after_ms: None,
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

    pub fn with_retry_after_ms(mut self, ms: u64) -> Self {
        self.retry_after_ms = Some(ms);
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
        // api_key=... fragments
        if i + 8 <= bytes.len() && out[i..].to_ascii_lowercase().starts_with("api_key=") {
            redacted.push_str("api_key=[REDACTED]");
            i += 8;
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
    parse_openai_error_body(body)
}

/// OpenAI-style: `{ "error": { "message", "code", "type" } }`.
pub fn parse_openai_error_body(body: &str) -> (Option<String>, Option<String>) {
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
    let code = err
        .and_then(|e| e.get("code"))
        .and_then(|c| {
            c.as_str()
                .map(|s| s.to_string())
                .or_else(|| c.as_i64().map(|n| n.to_string()))
        })
        .or_else(|| {
            err.and_then(|e| e.get("type"))
                .and_then(|t| t.as_str())
                .map(|s| s.to_string())
        });
    (message, code)
}

/// Anthropic-style: `{ "type":"error", "error": { "type", "message" } }`.
pub fn parse_anthropic_error_body(body: &str) -> (Option<String>, Option<String>) {
    let (message, openai_code) = parse_openai_error_body(body);
    let value: serde_json::Value = match serde_json::from_str(body) {
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

/// Gemini / Google: `{ "error": { "message", "status", "code" } }`.
pub fn parse_gemini_error_body(body: &str) -> (Option<String>, Option<String>) {
    let (message, openai_code) = parse_openai_error_body(body);
    let value: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return (message, openai_code),
    };
    let err = value.get("error");
    let code = err
        .and_then(|e| e.get("status"))
        .and_then(|s| s.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            err.and_then(|e| e.get("code")).and_then(|c| {
                c.as_str()
                    .map(|s| s.to_string())
                    .or_else(|| c.as_i64().map(|n| n.to_string()))
            })
        })
        .or(openai_code);
    (message, code)
}

/// Upgrade kind from body heuristics. Never downgrades Authentication (401).
pub fn upgrade_kind_from_body(
    kind: ProtocolErrorKind,
    status: u16,
    message: &str,
    provider_code: Option<&str>,
    raw_body: &str,
) -> ProtocolErrorKind {
    if status == 401 || kind == ProtocolErrorKind::Authentication {
        return ProtocolErrorKind::Authentication;
    }

    let haystack = format!(
        "{} {} {}",
        message.to_ascii_lowercase(),
        provider_code.unwrap_or("").to_ascii_lowercase(),
        raw_body.to_ascii_lowercase()
    );

    let looks_context = [
        "context_length",
        "context length",
        "maximum context",
        "max context",
        "token limit",
        "too many tokens",
        "max_tokens",
        "prompt is too long",
        "request too large",
        "context_window",
        "context window",
    ]
    .iter()
    .any(|needle| haystack.contains(needle));

    let looks_filter = [
        "content_filter",
        "content filter",
        "content_filtered",
        "content_policy",
        "safety",
        "blocked",
        "moderation",
        "responsibleai",
        "responsible_ai",
        "prohibited",
    ]
    .iter()
    .any(|needle| haystack.contains(needle));

    match kind {
        ProtocolErrorKind::InvalidRequest if looks_context => {
            ProtocolErrorKind::ContextLengthExceeded
        }
        ProtocolErrorKind::InvalidRequest | ProtocolErrorKind::PermissionDenied if looks_filter => {
            ProtocolErrorKind::ContentFilter
        }
        ProtocolErrorKind::InvalidRequest if looks_filter => ProtocolErrorKind::ContentFilter,
        other => other,
    }
}

/// Parse `Retry-After` into milliseconds.
///
/// Supports delta-seconds. HTTP-date is converted when parseable (IMF-fix GMT).
pub fn parse_retry_after(value: &str) -> Option<u64> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Ok(secs) = value.parse::<u64>() {
        return Some(secs.saturating_mul(1000));
    }
    parse_http_date_retry_after_ms(value)
}

fn parse_http_date_retry_after_ms(value: &str) -> Option<u64> {
    // IMF-fix: "Sun, 06 Nov 1994 08:49:37 GMT"
    let parts: Vec<&str> = value.split_whitespace().collect();
    if parts.len() != 6 {
        return None;
    }
    let day: u32 = parts[1].parse().ok()?;
    let month = match parts[2] {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    };
    let year: i32 = parts[3].parse().ok()?;
    let time_parts: Vec<&str> = parts[4].split(':').collect();
    if time_parts.len() != 3 || !parts[5].eq_ignore_ascii_case("GMT") {
        return None;
    }
    let hour: u32 = time_parts[0].parse().ok()?;
    let minute: u32 = time_parts[1].parse().ok()?;
    let second: u32 = time_parts[2].parse().ok()?;
    let target_secs = days_from_civil(year, month, day)?
        .checked_mul(86_400)?
        .checked_add(i64::from(hour) * 3600 + i64::from(minute) * 60 + i64::from(second))?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs() as i64;
    let delta = target_secs.saturating_sub(now).max(0) as u64;
    Some(delta.saturating_mul(1000))
}

/// Howard Hinnant civil-from-days inverse (days since 1970-01-01).
fn days_from_civil(year: i32, month: u32, day: u32) -> Option<i64> {
    if !(1..=12).contains(&month) || day == 0 || day > 31 {
        return None;
    }
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u32;
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(i64::from(era) * 146_097 + i64::from(doe) - 719_468)
}

fn request_id_from_headers(headers: &HeaderMap, family: ErrorBodyFamily) -> Option<String> {
    let candidates: &[&str] = match family {
        ErrorBodyFamily::OpenAi => &["x-request-id"],
        ErrorBodyFamily::Anthropic => &["request-id", "x-request-id"],
        ErrorBodyFamily::Gemini => &["x-goog-request-id", "x-request-id"],
    };
    for name in candidates {
        if let Some(value) = headers
            .get(*name)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Some(value.to_string());
        }
    }
    None
}

fn retry_after_from_headers(headers: &HeaderMap) -> Option<u64> {
    headers
        .get(RETRY_AFTER)
        .or_else(|| headers.get("retry-after"))
        .and_then(|v| v.to_str().ok())
        .and_then(parse_retry_after)
}

/// Build a [`ProtocolError`] from an HTTP error response body and headers.
pub fn map_http_error(
    status: u16,
    headers: &HeaderMap,
    body: &[u8],
    family: ErrorBodyFamily,
) -> ProtocolError {
    let limited = if body.len() > ERROR_BODY_LIMIT {
        &body[..ERROR_BODY_LIMIT]
    } else {
        body
    };
    let body_str = String::from_utf8_lossy(limited);
    let (message, code) = match family {
        ErrorBodyFamily::OpenAi => parse_openai_error_body(&body_str),
        ErrorBodyFamily::Anthropic => parse_anthropic_error_body(&body_str),
        ErrorBodyFamily::Gemini => parse_gemini_error_body(&body_str),
    };
    let message = message.unwrap_or_else(|| {
        if body_str.trim().is_empty() {
            format!("服务商返回 HTTP {status}")
        } else {
            redact_secrets(body_str.trim())
        }
    });
    let base_kind = kind_from_status(status);
    let kind = upgrade_kind_from_body(
        base_kind,
        status,
        &message,
        code.as_deref(),
        body_str.as_ref(),
    );
    let mut err = ProtocolError::new(kind, message).with_http_status(status);
    if let Some(code) = code {
        err = err.with_provider_code(code);
    }
    if let Some(id) = request_id_from_headers(headers, family) {
        err = err.with_request_id(id);
    }
    if let Some(ms) = retry_after_from_headers(headers) {
        err = err.with_retry_after_ms(ms);
    }
    err
}

pub const ERROR_BODY_LIMIT: usize = 4 * 1024;

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderName, HeaderValue};

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
        assert!(!ProtocolErrorKind::ContextLengthExceeded.retryable());
        assert!(!ProtocolErrorKind::ContentFilter.retryable());
        assert_eq!(kind_from_status(400), ProtocolErrorKind::InvalidRequest);
        assert_eq!(kind_from_status(422), ProtocolErrorKind::InvalidRequest);
        assert_eq!(kind_from_status(408), ProtocolErrorKind::Timeout);
    }

    #[test]
    fn redacts_keys_in_messages() {
        let raw = r#"Incorrect API key provided: sk-abc123DEF. Authorization: Bearer sk-abc123DEF api_key=secret_val"#;
        let redacted = redact_secrets(raw);
        assert!(!redacted.contains("sk-abc123DEF"));
        assert!(!redacted.contains("secret_val"));
        assert!(redacted.contains("REDACTED"));
    }

    #[test]
    fn parses_openai_error_json() {
        let body = r#"{"error":{"message":"bad sk-secret-key","type":"invalid_request_error","code":"invalid_api_key"}}"#;
        let (msg, code) = parse_openai_error_body(body);
        assert_eq!(code.as_deref(), Some("invalid_api_key"));
        assert!(msg.unwrap().contains("REDACTED"));
    }

    #[test]
    fn parses_anthropic_and_gemini_error_json() {
        let anthropic = r#"{"type":"error","error":{"type":"authentication_error","message":"bad sk-secret-key"}}"#;
        let (msg, code) = parse_anthropic_error_body(anthropic);
        assert_eq!(code.as_deref(), Some("authentication_error"));
        assert!(msg.unwrap().contains("REDACTED"));

        let gemini =
            r#"{"error":{"code":401,"message":"bad key AIza-secret","status":"UNAUTHENTICATED"}}"#;
        let (msg, code) = parse_gemini_error_body(gemini);
        assert_eq!(code.as_deref(), Some("UNAUTHENTICATED"));
        assert!(msg.is_some());
    }

    #[test]
    fn upgrades_context_and_filter_without_downgrading_auth() {
        let kind = upgrade_kind_from_body(
            ProtocolErrorKind::InvalidRequest,
            400,
            "This model's maximum context length is 8k tokens",
            Some("context_length_exceeded"),
            "",
        );
        assert_eq!(kind, ProtocolErrorKind::ContextLengthExceeded);

        let kind = upgrade_kind_from_body(
            ProtocolErrorKind::PermissionDenied,
            403,
            "content_filter triggered",
            None,
            "",
        );
        assert_eq!(kind, ProtocolErrorKind::ContentFilter);

        let kind = upgrade_kind_from_body(
            ProtocolErrorKind::Authentication,
            401,
            "context length exceeded but actually auth",
            None,
            "",
        );
        assert_eq!(kind, ProtocolErrorKind::Authentication);
    }

    #[test]
    fn parses_retry_after_delta_seconds() {
        assert_eq!(parse_retry_after("12"), Some(12_000));
        assert_eq!(parse_retry_after(" 0 "), Some(0));
        assert_eq!(parse_retry_after(""), None);
    }

    #[test]
    fn map_http_error_sets_retry_after_and_request_id() {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static("5"));
        headers.insert(
            HeaderName::from_static("x-request-id"),
            HeaderValue::from_static("req_123"),
        );
        let body = br#"{"error":{"message":"slow down","code":"rate_limit_exceeded"}}"#;
        let err = map_http_error(429, &headers, body, ErrorBodyFamily::OpenAi);
        assert_eq!(err.kind, ProtocolErrorKind::RateLimited);
        assert_eq!(err.retry_after_ms, Some(5_000));
        assert_eq!(err.request_id.as_deref(), Some("req_123"));
        assert!(err.retryable);
        assert_eq!(err.provider_code.as_deref(), Some("rate_limit_exceeded"));
    }

    #[test]
    fn map_http_error_context_length_fixture() {
        let headers = HeaderMap::new();
        let body = br#"{"error":{"message":"maximum context length exceeded","code":"context_length_exceeded"}}"#;
        let err = map_http_error(400, &headers, body, ErrorBodyFamily::OpenAi);
        assert_eq!(err.kind, ProtocolErrorKind::ContextLengthExceeded);
        assert!(!err.retryable);
    }
}
