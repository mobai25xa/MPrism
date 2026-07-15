//! Decode Anthropic Messages SSE JSON into StreamEvent.

use crate::error::{redact_secrets, ProtocolError, ProtocolErrorKind};
use crate::finish::anthropic_messages as map_finish;
use crate::types::StreamEvent;
use crate::usage::from_anthropic_usage;
use serde_json::Value;

#[derive(Debug, Default)]
struct PendingToolUse {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
    finished: bool,
}

/// Per-stream decoder state for Anthropic Messages events.
#[derive(Debug, Default)]
pub struct EventDecodeState {
    pub completed: bool,
    pub stop_reason: Option<String>,
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    last_usage: Option<Value>,
    usage_emitted: bool,
    /// Keyed by content block index.
    pending_tools: std::collections::HashMap<u32, PendingToolUse>,
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
        "content_block_start" => Ok(decode_content_block_start(state, &value)),
        "content_block_delta" => Ok(decode_content_block_delta(state, &value)),
        "content_block_stop" => Ok(decode_content_block_stop(state, &value)),
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
        // Lifecycle / unknown: ignore for forward compatibility.
        "ping" => Ok(Vec::new()),
        _ => Ok(Vec::new()),
    }
}

fn block_index(value: &Value) -> u32 {
    value
        .get("index")
        .and_then(|v| v.as_u64())
        .map(|n| n as u32)
        .unwrap_or(0)
}

fn decode_content_block_start(state: &mut EventDecodeState, value: &Value) -> Vec<StreamEvent> {
    let block = match value.get("content_block") {
        Some(b) => b,
        None => return Vec::new(),
    };
    if block.get("type").and_then(|t| t.as_str()) != Some("tool_use") {
        return Vec::new();
    }
    let index = block_index(value);
    let entry = state.pending_tools.entry(index).or_default();
    entry.id = block
        .get("id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    entry.name = block
        .get("name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    // partial_json may start empty; input may be present as object on non-stream paths.
    if let Some(input) = block.get("input") {
        if !input.is_null() && entry.arguments.is_empty() {
            if let Ok(s) = serde_json::to_string(input) {
                if s != "{}" && s != "null" {
                    entry.arguments = s;
                }
            }
        }
    }
    vec![StreamEvent::ToolCallDelta {
        id: entry.id.clone(),
        name: entry.name.clone(),
        arguments_delta: String::new(),
        index: Some(index),
    }]
}

fn decode_content_block_delta(state: &mut EventDecodeState, value: &Value) -> Vec<StreamEvent> {
    let delta = match value.get("delta") {
        Some(d) => d,
        None => return Vec::new(),
    };
    let delta_type = delta.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match delta_type {
        "text_delta" => {
            if let Some(text) = non_empty_str(delta.get("text")) {
                return vec![StreamEvent::ContentDelta { text }];
            }
            Vec::new()
        }
        "thinking_delta" => {
            if let Some(text) = non_empty_str(delta.get("thinking")) {
                return vec![StreamEvent::ReasoningDelta { text }];
            }
            Vec::new()
        }
        "input_json_delta" => {
            let index = block_index(value);
            let partial = delta
                .get("partial_json")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let entry = state.pending_tools.entry(index).or_default();
            if !partial.is_empty() {
                entry.arguments.push_str(partial);
            }
            vec![StreamEvent::ToolCallDelta {
                id: entry.id.clone(),
                name: entry.name.clone(),
                arguments_delta: partial.to_string(),
                index: Some(index),
            }]
        }
        _ => Vec::new(),
    }
}

fn decode_content_block_stop(state: &mut EventDecodeState, value: &Value) -> Vec<StreamEvent> {
    let index = block_index(value);
    finish_tool_at(state, index)
}

fn finish_tool_at(state: &mut EventDecodeState, index: u32) -> Vec<StreamEvent> {
    let Some(entry) = state.pending_tools.get_mut(&index) else {
        return Vec::new();
    };
    if entry.finished {
        return Vec::new();
    }
    entry.finished = true;
    let id = entry.id.clone().unwrap_or_default();
    let name = entry.name.clone().unwrap_or_default();
    let arguments = entry.arguments.clone();
    if id.is_empty() && name.is_empty() && arguments.is_empty() {
        return Vec::new();
    }
    vec![StreamEvent::ToolCallFinished {
        id,
        name,
        arguments,
        index: Some(index),
    }]
}

fn finish_all_tools(state: &mut EventDecodeState) -> Vec<StreamEvent> {
    let mut indices: Vec<u32> = state.pending_tools.keys().copied().collect();
    indices.sort_unstable();
    let mut events = Vec::new();
    for index in indices {
        events.extend(finish_tool_at(state, index));
    }
    events
}

fn terminal_events(state: &mut EventDecodeState) -> Vec<StreamEvent> {
    let mut events = finish_all_tools(state);
    if !state.usage_emitted {
        if let Some(usage) = from_anthropic_usage(
            state.input_tokens,
            state.output_tokens,
            state.last_usage.as_ref(),
        ) {
            state.usage_emitted = true;
            events.push(StreamEvent::Usage(usage));
        }
    }
    let finish_reason = map_finish(state.stop_reason.as_deref());
    events.push(StreamEvent::Completed { finish_reason });
    events
}

fn merge_usage(state: &mut EventDecodeState, usage: &Value) {
    if let Some(n) = as_u32(usage.get("input_tokens")) {
        state.input_tokens = Some(n);
    }
    if let Some(n) = as_u32(usage.get("output_tokens")) {
        state.output_tokens = Some(n);
    }
    state.last_usage = Some(usage.clone());
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
    use crate::types::FinishReason;

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
    fn tool_use_input_json_delta_stream() {
        let mut state = EventDecodeState::default();
        let mut events = Vec::new();
        events.extend(
            decode_sse_data(
                r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"tu_1","name":"lookup","input":{}}}"#,
                &mut state,
            )
            .unwrap(),
        );
        events.extend(
            decode_sse_data(
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"q\":"}}"#,
                &mut state,
            )
            .unwrap(),
        );
        events.extend(
            decode_sse_data(
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"1}"}}"#,
                &mut state,
            )
            .unwrap(),
        );
        events.extend(
            decode_sse_data(r#"{"type":"content_block_stop","index":1}"#, &mut state).unwrap(),
        );
        events.extend(
            decode_sse_data(
                r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
                &mut state,
            )
            .unwrap(),
        );
        events.extend(decode_sse_data(r#"{"type":"message_stop"}"#, &mut state).unwrap());

        assert!(events.iter().any(|e| matches!(
            e,
            StreamEvent::ToolCallFinished {
                id,
                name,
                arguments,
                index: Some(1)
            } if id == "tu_1" && name == "lookup" && arguments == r#"{"q":1}"#
        )));
        assert!(matches!(
            events.last(),
            Some(StreamEvent::Completed {
                finish_reason: FinishReason::ToolCalls
            })
        ));
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
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":2,"output_tokens_details":{"thinking_tokens":4}}}"#,
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
                assert_eq!(u.reasoning_tokens, Some(4));
            }
            other => panic!("expected usage, got {other:?}"),
        }
        match &events[1] {
            StreamEvent::Completed { finish_reason } => {
                assert_eq!(finish_reason, &FinishReason::Stop);
            }
            other => panic!("expected completed, got {other:?}"),
        }
        assert!(state.completed);
    }

    #[test]
    fn stop_reason_table() {
        for (reason, expected) in [
            ("max_tokens", FinishReason::Length),
            ("tool_use", FinishReason::ToolCalls),
            ("stop_sequence", FinishReason::Stop),
            ("refusal", FinishReason::ContentFilter),
        ] {
            let mut state = EventDecodeState::default();
            decode_sse_data(
                &format!(r#"{{"type":"message_delta","delta":{{"stop_reason":"{reason}"}}}}"#),
                &mut state,
            )
            .unwrap();
            let events = decode_sse_data(r#"{"type":"message_stop"}"#, &mut state).unwrap();
            assert_eq!(
                events,
                vec![StreamEvent::Completed {
                    finish_reason: expected
                }]
            );
        }
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
