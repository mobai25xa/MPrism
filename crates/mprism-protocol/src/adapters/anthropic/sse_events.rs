//! Decode Anthropic Messages SSE JSON into StreamEvent.

use crate::error::{redact_secrets, ProtocolError, ProtocolErrorKind};
use crate::types::{StreamEvent, TokenUsage};
use serde_json::Value;

/// Per-stream decoder state for Anthropic Messages events.
#[derive(Debug, Default)]
pub struct EventDecodeState {
    pub completed: bool,
    pub stop_reason: Option<String>,
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    usage_emitted: bool,
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

    let value: Value = serde_json::from_str(trimmed).map_err(|err| {
        ProtocolError::new(
            ProtocolErrorKind::Decode,
            format!("SSE JSON 解码失败: {err}"),
        )
    })?;

    if is_error_payload(&value) {
        return Err(error_from_json(&value));
    }

    let event_type = value.get("type").and_then(|t| t.as_str()).unwrap_or("");

    match event_type {
        "message_start" => {
            if let Some(usage) = value.get("message").and_then(|m| m.get("usage")) {
                merge_usage(state, usage);
            }
            Ok(Vec::new())
        }
        "content_block_delta" => decode_content_block_delta(&value),
        "message_delta" => {
            if let Some(reason) = value
                .get("delta")
                .and_then(|d| d.get("stop_reason"))
                .and_then(|r| r.as_str())
            {
                state.stop_reason = Some(reason.to_string());
            }
            if let Some(usage) = value.get("usage") {
                merge_usage(state, usage);
            }
            Ok(Vec::new())
        }
        "message_stop" => {
            if state.completed {
                return Ok(Vec::new());
            }
            state.completed = true;
            Ok(terminal_events(state))
        }
        // Lifecycle / tools / unknown: ignore for forward compatibility.
        "content_block_start" | "content_block_stop" | "ping" => Ok(Vec::new()),
        _ => Ok(Vec::new()),
    }
}

fn decode_content_block_delta(value: &Value) -> Result<Vec<StreamEvent>, ProtocolError> {
    let delta = match value.get("delta") {
        Some(d) => d,
        None => return Ok(Vec::new()),
    };
    let delta_type = delta.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match delta_type {
        "text_delta" => {
            if let Some(text) = non_empty_str(delta.get("text")) {
                return Ok(vec![StreamEvent::ContentDelta { text }]);
            }
            Ok(Vec::new())
        }
        "thinking_delta" => {
            if let Some(text) = non_empty_str(delta.get("thinking")) {
                return Ok(vec![StreamEvent::ReasoningDelta { text }]);
            }
            Ok(Vec::new())
        }
        _ => Ok(Vec::new()),
    }
}

fn terminal_events(state: &mut EventDecodeState) -> Vec<StreamEvent> {
    let mut events = Vec::new();
    if !state.usage_emitted {
        if let Some(usage) = usage_from_state(state) {
            state.usage_emitted = true;
            events.push(StreamEvent::Usage(usage));
        }
    }
    let finish_reason = state
        .stop_reason
        .clone()
        .or_else(|| Some("end_turn".to_string()));
    events.push(StreamEvent::Completed { finish_reason });
    events
}

fn usage_from_state(state: &EventDecodeState) -> Option<TokenUsage> {
    if state.input_tokens.is_none() && state.output_tokens.is_none() {
        return None;
    }
    let total = match (state.input_tokens, state.output_tokens) {
        (Some(i), Some(o)) => Some(i.saturating_add(o)),
        _ => None,
    };
    Some(TokenUsage {
        prompt_tokens: state.input_tokens,
        completion_tokens: state.output_tokens,
        total_tokens: total,
    })
}

fn merge_usage(state: &mut EventDecodeState, usage: &Value) {
    if let Some(n) = as_u32(usage.get("input_tokens")) {
        state.input_tokens = Some(n);
    }
    if let Some(n) = as_u32(usage.get("output_tokens")) {
        state.output_tokens = Some(n);
    }
}

fn as_u32(value: Option<&Value>) -> Option<u32> {
    value.and_then(|v| v.as_u64()).map(|n| n as u32)
}

fn non_empty_str(value: Option<&Value>) -> Option<String> {
    value
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

fn is_error_payload(value: &Value) -> bool {
    if value.get("type").and_then(|t| t.as_str()) == Some("error") {
        return true;
    }
    value.get("error").is_some() && value.get("type").is_none()
}

fn error_from_json(value: &Value) -> ProtocolError {
    let err = value.get("error");
    let message = err
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
        .or_else(|| value.get("message").and_then(|m| m.as_str()))
        .unwrap_or("服务商返回错误");
    let code = err
        .and_then(|e| e.get("type"))
        .and_then(|t| t.as_str())
        .or_else(|| err.and_then(|e| e.get("code")).and_then(|c| c.as_str()))
        .map(|s| s.to_string());
    let mut out = ProtocolError::new(
        ProtocolErrorKind::ProviderUnavailable,
        redact_secrets(message),
    );
    if let Some(code) = code {
        out = out.with_provider_code(code);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_and_thinking_deltas() {
        let mut state = EventDecodeState::default();
        let events = decode_sse_data(
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}}"#,
            &mut state,
        )
        .unwrap();
        assert_eq!(
            events,
            vec![StreamEvent::ContentDelta { text: "Hi".into() }]
        );

        let events = decode_sse_data(
            r#"{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"plan"}}"#,
            &mut state,
        )
        .unwrap();
        assert_eq!(
            events,
            vec![StreamEvent::ReasoningDelta {
                text: "plan".into()
            }]
        );
    }

    #[test]
    fn message_stop_emits_usage_and_completed() {
        let mut state = EventDecodeState::default();
        decode_sse_data(
            r#"{"type":"message_start","message":{"usage":{"input_tokens":3,"output_tokens":0}}}"#,
            &mut state,
        )
        .unwrap();
        decode_sse_data(
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":2}}"#,
            &mut state,
        )
        .unwrap();
        let events = decode_sse_data(r#"{"type":"message_stop"}"#, &mut state).unwrap();
        assert_eq!(events.len(), 2);
        match &events[0] {
            StreamEvent::Usage(u) => {
                assert_eq!(u.prompt_tokens, Some(3));
                assert_eq!(u.completion_tokens, Some(2));
                assert_eq!(u.total_tokens, Some(5));
            }
            other => panic!("expected usage, got {other:?}"),
        }
        match &events[1] {
            StreamEvent::Completed { finish_reason } => {
                assert_eq!(finish_reason.as_deref(), Some("end_turn"));
            }
            other => panic!("expected completed, got {other:?}"),
        }
        assert!(state.completed);
    }

    #[test]
    fn ignores_unknown_types() {
        let mut state = EventDecodeState::default();
        let events =
            decode_sse_data(r#"{"type":"content_block_start","index":0}"#, &mut state).unwrap();
        assert!(events.is_empty());
        let events = decode_sse_data(r#"{"type":"future_event","x":1}"#, &mut state).unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn stream_error_payload() {
        let mut state = EventDecodeState::default();
        let err = decode_sse_data(
            r#"{"type":"error","error":{"type":"overloaded_error","message":"busy sk-leaked"}}"#,
            &mut state,
        )
        .unwrap_err();
        assert_eq!(err.provider_code.as_deref(), Some("overloaded_error"));
        assert!(!err.message.contains("sk-leaked"));
    }
}
