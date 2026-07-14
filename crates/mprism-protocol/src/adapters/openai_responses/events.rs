//! Decode OpenAI Responses semantic SSE JSON into StreamEvent.
//!
//! Independent from Chat Completions `chat.completion.chunk` decoding.

use crate::error::{parse_provider_error_body, ProtocolError, ProtocolErrorKind};
use crate::types::{StreamEvent, TokenUsage};
use serde_json::Value;

/// Per-stream decoder state for Responses semantic events.
#[derive(Debug, Default)]
pub struct EventDecodeState {
    pub completed: bool,
}

/// Decode one SSE `data` payload (JSON with `type` field).
pub fn decode_sse_data(
    data: &str,
    state: &mut EventDecodeState,
) -> Result<Vec<StreamEvent>, ProtocolError> {
    let trimmed = data.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    // Responses does not use [DONE]; tolerate it if a proxy injects it.
    if trimmed == "[DONE]" {
        if state.completed {
            return Ok(Vec::new());
        }
        state.completed = true;
        return Ok(vec![StreamEvent::Completed {
            finish_reason: Some("stop".into()),
        }]);
    }

    let value: Value = serde_json::from_str(trimmed).map_err(|err| {
        ProtocolError::new(
            ProtocolErrorKind::Decode,
            format!("SSE JSON 解码失败: {err}"),
        )
    })?;

    // Some gateways embed error objects without type.
    if value.get("error").is_some() && value.get("type").is_none() {
        return Err(error_from_json(&value, trimmed));
    }

    let event_type = value.get("type").and_then(|t| t.as_str()).unwrap_or("");

    match event_type {
        "response.output_text.delta" => {
            if let Some(text) = non_empty_str(value.get("delta")) {
                return Ok(vec![StreamEvent::ContentDelta { text }]);
            }
            Ok(Vec::new())
        }
        "response.reasoning_summary_text.delta"
        | "response.reasoning_text.delta"
        | "response.reasoning.delta" => {
            if let Some(text) = non_empty_str(value.get("delta")) {
                return Ok(vec![StreamEvent::ReasoningDelta { text }]);
            }
            Ok(Vec::new())
        }
        "response.completed" => {
            if state.completed {
                return Ok(Vec::new());
            }
            state.completed = true;
            Ok(events_from_terminal_response(value.get("response"), "stop"))
        }
        "response.incomplete" => {
            if state.completed {
                return Ok(Vec::new());
            }
            state.completed = true;
            let reason = value
                .get("response")
                .and_then(|r| r.get("incomplete_details"))
                .and_then(|d| d.get("reason"))
                .and_then(|r| r.as_str())
                .unwrap_or("incomplete");
            Ok(events_from_terminal_response(value.get("response"), reason))
        }
        "response.failed" => Err(failed_from_response(value.get("response"))),
        "error" => Err(error_from_stream_event(&value)),
        // Lifecycle / tools / unknown: ignore (backward compatible).
        _ => Ok(Vec::new()),
    }
}

fn events_from_terminal_response(
    response: Option<&Value>,
    finish_reason: &str,
) -> Vec<StreamEvent> {
    let mut events = Vec::new();
    if let Some(usage) = response.and_then(|r| r.get("usage")) {
        let prompt = usage
            .get("input_tokens")
            .or_else(|| usage.get("prompt_tokens"))
            .and_then(|v| v.as_u64())
            .map(|n| n as u32);
        let completion = usage
            .get("output_tokens")
            .or_else(|| usage.get("completion_tokens"))
            .and_then(|v| v.as_u64())
            .map(|n| n as u32);
        let total = usage
            .get("total_tokens")
            .and_then(|v| v.as_u64())
            .map(|n| n as u32);
        if prompt.is_some() || completion.is_some() || total.is_some() {
            events.push(StreamEvent::Usage(TokenUsage {
                prompt_tokens: prompt,
                completion_tokens: completion,
                total_tokens: total,
            }));
        }
    }
    events.push(StreamEvent::Completed {
        finish_reason: Some(finish_reason.to_string()),
    });
    events
}

fn failed_from_response(response: Option<&Value>) -> ProtocolError {
    if let Some(err_val) = response.and_then(|r| r.get("error")) {
        let message = err_val
            .get("message")
            .and_then(|m| m.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "Response 失败".into());
        let code = err_val
            .get("code")
            .and_then(|c| c.as_str())
            .map(|s| s.to_string());
        let mut err = ProtocolError::new(
            ProtocolErrorKind::ProviderUnavailable,
            crate::error::redact_secrets(&message),
        );
        if let Some(code) = code {
            err = err.with_provider_code(code);
        }
        return err;
    }
    ProtocolError::new(ProtocolErrorKind::ProviderUnavailable, "Response 失败")
}

fn error_from_stream_event(value: &Value) -> ProtocolError {
    let message = value
        .get("message")
        .and_then(|m| m.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "服务商在流中返回错误".into());
    let code = value
        .get("code")
        .and_then(|c| c.as_str())
        .map(|s| s.to_string());
    let mut err = ProtocolError::new(
        ProtocolErrorKind::ProviderUnavailable,
        crate::error::redact_secrets(&message),
    );
    if let Some(code) = code {
        err = err.with_provider_code(code);
    }
    err
}

fn error_from_json(_value: &Value, raw: &str) -> ProtocolError {
    let (msg, code) = parse_provider_error_body(raw);
    let mut err = ProtocolError::new(
        ProtocolErrorKind::ProviderUnavailable,
        msg.unwrap_or_else(|| "服务商在流中返回错误".into()),
    );
    if let Some(code) = code {
        err = err.with_provider_code(code);
    }
    err
}

fn non_empty_str(value: Option<&Value>) -> Option<String> {
    value.and_then(|v| v.as_str()).and_then(|s| {
        if s.is_empty() {
            None
        } else {
            Some(s.to_string())
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_text_reasoning_completed_usage() {
        let mut state = EventDecodeState::default();
        let mut events = Vec::new();
        events.extend(
            decode_sse_data(
                r#"{"type":"response.created","response":{"id":"resp_1"}}"#,
                &mut state,
            )
            .unwrap(),
        );
        events.extend(
            decode_sse_data(
                r#"{"type":"response.reasoning_summary_text.delta","delta":"think "}"#,
                &mut state,
            )
            .unwrap(),
        );
        events.extend(
            decode_sse_data(
                r#"{"type":"response.output_text.delta","delta":"Hel"}"#,
                &mut state,
            )
            .unwrap(),
        );
        events.extend(
            decode_sse_data(
                r#"{"type":"response.output_text.delta","delta":"lo"}"#,
                &mut state,
            )
            .unwrap(),
        );
        events.extend(
            decode_sse_data(
                r#"{"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":3,"output_tokens":2,"total_tokens":5}}}"#,
                &mut state,
            )
            .unwrap(),
        );

        assert!(events
            .iter()
            .any(|e| matches!(e, StreamEvent::ReasoningDelta { text } if text == "think ")));
        assert_eq!(
            events
                .iter()
                .filter_map(|e| match e {
                    StreamEvent::ContentDelta { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<String>(),
            "Hello"
        );
        assert!(matches!(
            events.last(),
            Some(StreamEvent::Completed {
                finish_reason: Some(r)
            }) if r == "stop"
        ));
        assert!(state.completed);
        assert!(
            decode_sse_data(r#"{"type":"response.completed","response":{}}"#, &mut state)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn incomplete_and_error() {
        let mut state = EventDecodeState::default();
        let events = decode_sse_data(
            r#"{"type":"response.incomplete","response":{"incomplete_details":{"reason":"max_output_tokens"},"usage":{"input_tokens":1,"output_tokens":2,"total_tokens":3}}}"#,
            &mut state,
        )
        .unwrap();
        assert!(matches!(
            events.last(),
            Some(StreamEvent::Completed {
                finish_reason: Some(r)
            }) if r == "max_output_tokens"
        ));

        let mut state = EventDecodeState::default();
        let err = decode_sse_data(
            r#"{"type":"error","code":"server_error","message":"boom sk-secret"}"#,
            &mut state,
        )
        .unwrap_err();
        assert_eq!(err.kind, ProtocolErrorKind::ProviderUnavailable);
        assert!(!err.message.contains("sk-secret"));
    }

    #[test]
    fn failed_event() {
        let mut state = EventDecodeState::default();
        let err = decode_sse_data(
            r#"{"type":"response.failed","response":{"error":{"code":"server_error","message":"fail"}}}"#,
            &mut state,
        )
        .unwrap_err();
        assert_eq!(err.provider_code.as_deref(), Some("server_error"));
    }

    #[test]
    fn malformed_json() {
        let mut state = EventDecodeState::default();
        let err = decode_sse_data("{not-json", &mut state).unwrap_err();
        assert_eq!(err.kind, ProtocolErrorKind::Decode);
    }
}
