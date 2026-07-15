//! Decode OpenAI Responses semantic SSE JSON into StreamEvent.
//!
//! Independent from Chat Completions `chat.completion.chunk` decoding.

use crate::error::{parse_provider_error_body, ProtocolError, ProtocolErrorKind};
use crate::finish::openai_responses as map_finish;
use crate::types::StreamEvent;
use crate::usage::from_openai_responses_usage;
use serde_json::Value;

/// Accumulator for function-call argument streaming.
#[derive(Debug, Default)]
struct PendingFunctionCall {
    call_id: Option<String>,
    name: Option<String>,
    arguments: String,
    finished: bool,
}

/// Per-stream decoder state for Responses semantic events.
#[derive(Debug, Default)]
pub struct EventDecodeState {
    pub completed: bool,
    /// Keyed by output item index when present; otherwise a single slot `0`.
    pending_calls: std::collections::HashMap<u32, PendingFunctionCall>,
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
            finish_reason: map_finish(Some("completed")),
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
        // Function call streaming (independent of Chat Completions chunk decoder).
        "response.output_item.added" => Ok(handle_output_item_added(state, &value)),
        "response.function_call_arguments.delta" => {
            Ok(handle_function_call_arguments_delta(state, &value))
        }
        "response.function_call_arguments.done" => {
            Ok(handle_function_call_arguments_done(state, &value))
        }
        "response.output_item.done" => Ok(handle_output_item_done(state, &value)),
        "response.completed" => {
            if state.completed {
                return Ok(Vec::new());
            }
            state.completed = true;
            let mut events = finish_all_pending_calls(state);
            events.extend(events_from_terminal_response(value.get("response"), "stop"));
            // If response output contains function_call items, prefer ToolCalls finish.
            if response_has_function_call(value.get("response")) {
                if let Some(StreamEvent::Completed { finish_reason }) = events.last_mut() {
                    *finish_reason = map_finish(Some("tool_use"));
                }
            }
            Ok(events)
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
            let mut events = finish_all_pending_calls(state);
            events.extend(events_from_terminal_response(value.get("response"), reason));
            Ok(events)
        }
        "response.failed" => Err(failed_from_response(value.get("response"))),
        "error" => Err(error_from_stream_event(&value)),
        // Lifecycle / unknown: ignore (backward compatible).
        _ => Ok(Vec::new()),
    }
}

fn item_index(value: &Value) -> u32 {
    value
        .get("output_index")
        .or_else(|| value.get("index"))
        .and_then(|v| v.as_u64())
        .map(|n| n as u32)
        .unwrap_or(0)
}

fn handle_output_item_added(state: &mut EventDecodeState, value: &Value) -> Vec<StreamEvent> {
    let item = match value.get("item") {
        Some(i) => i,
        None => return Vec::new(),
    };
    let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
    if item_type != "function_call" {
        return Vec::new();
    }
    let index = item_index(value);
    let entry = state.pending_calls.entry(index).or_default();
    if let Some(id) = item
        .get("call_id")
        .or_else(|| item.get("id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        entry.call_id = Some(id.to_string());
    }
    if let Some(name) = item
        .get("name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        entry.name = Some(name.to_string());
    }
    vec![StreamEvent::ToolCallDelta {
        id: entry.call_id.clone(),
        name: entry.name.clone(),
        arguments_delta: String::new(),
        index: Some(index),
    }]
}

fn handle_function_call_arguments_delta(
    state: &mut EventDecodeState,
    value: &Value,
) -> Vec<StreamEvent> {
    let index = item_index(value);
    let delta = value.get("delta").and_then(|v| v.as_str()).unwrap_or("");
    let entry = state.pending_calls.entry(index).or_default();
    if !delta.is_empty() {
        entry.arguments.push_str(delta);
    }
    if let Some(id) = value
        .get("call_id")
        .or_else(|| value.get("item_id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        if entry.call_id.is_none() {
            entry.call_id = Some(id.to_string());
        }
    }
    vec![StreamEvent::ToolCallDelta {
        id: entry.call_id.clone(),
        name: entry.name.clone(),
        arguments_delta: delta.to_string(),
        index: Some(index),
    }]
}

fn handle_function_call_arguments_done(
    state: &mut EventDecodeState,
    value: &Value,
) -> Vec<StreamEvent> {
    let index = item_index(value);
    let entry = state.pending_calls.entry(index).or_default();
    if let Some(args) = value.get("arguments").and_then(|v| v.as_str()) {
        // Done may carry the full arguments string.
        if entry.arguments.is_empty() {
            entry.arguments = args.to_string();
        }
    }
    if let Some(id) = value
        .get("call_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        entry.call_id = Some(id.to_string());
    }
    if let Some(name) = value
        .get("name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        entry.name = Some(name.to_string());
    }
    finish_call_at(state, index)
}

fn handle_output_item_done(state: &mut EventDecodeState, value: &Value) -> Vec<StreamEvent> {
    let item = match value.get("item") {
        Some(i) => i,
        None => return Vec::new(),
    };
    if item.get("type").and_then(|t| t.as_str()) != Some("function_call") {
        return Vec::new();
    }
    let index = item_index(value);
    let entry = state.pending_calls.entry(index).or_default();
    if let Some(id) = item
        .get("call_id")
        .or_else(|| item.get("id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        entry.call_id = Some(id.to_string());
    }
    if let Some(name) = item
        .get("name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        entry.name = Some(name.to_string());
    }
    if let Some(args) = item.get("arguments").and_then(|v| v.as_str()) {
        if entry.arguments.is_empty() {
            entry.arguments = args.to_string();
        }
    }
    finish_call_at(state, index)
}

fn finish_call_at(state: &mut EventDecodeState, index: u32) -> Vec<StreamEvent> {
    let Some(entry) = state.pending_calls.get_mut(&index) else {
        return Vec::new();
    };
    if entry.finished {
        return Vec::new();
    }
    entry.finished = true;
    let id = entry.call_id.clone().unwrap_or_default();
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

fn finish_all_pending_calls(state: &mut EventDecodeState) -> Vec<StreamEvent> {
    let mut indices: Vec<u32> = state.pending_calls.keys().copied().collect();
    indices.sort_unstable();
    let mut events = Vec::new();
    for index in indices {
        events.extend(finish_call_at(state, index));
    }
    events
}

fn response_has_function_call(response: Option<&Value>) -> bool {
    response
        .and_then(|r| r.get("output"))
        .and_then(|o| o.as_array())
        .map(|items| {
            items
                .iter()
                .any(|item| item.get("type").and_then(|t| t.as_str()) == Some("function_call"))
        })
        .unwrap_or(false)
}

fn events_from_terminal_response(
    response: Option<&Value>,
    finish_reason: &str,
) -> Vec<StreamEvent> {
    let mut events = Vec::new();
    if let Some(usage) = response.and_then(|r| r.get("usage")) {
        if let Some(usage) = from_openai_responses_usage(usage) {
            events.push(StreamEvent::Usage(usage));
        }
    }
    events.push(StreamEvent::Completed {
        finish_reason: map_finish(Some(finish_reason)),
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
    use crate::types::FinishReason;

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
                finish_reason: FinishReason::Stop
            })
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
            r#"{"type":"response.incomplete","response":{"incomplete_details":{"reason":"max_output_tokens"},"usage":{"input_tokens":1,"output_tokens":2,"total_tokens":3,"output_tokens_details":{"reasoning_tokens":4}}}}"#,
            &mut state,
        )
        .unwrap();
        assert!(matches!(
            events.first(),
            Some(StreamEvent::Usage(u)) if u.reasoning_tokens == Some(4)
        ));
        assert!(matches!(
            events.last(),
            Some(StreamEvent::Completed {
                finish_reason: FinishReason::Length
            })
        ));

        for (reason, expected) in [
            ("content_filter", FinishReason::ContentFilter),
            ("tool_use", FinishReason::ToolCalls),
            ("custom_reason", FinishReason::Other("custom_reason".into())),
        ] {
            let mut state = EventDecodeState::default();
            let events = decode_sse_data(
                &format!(
                    r#"{{"type":"response.incomplete","response":{{"incomplete_details":{{"reason":"{reason}"}}}}}}"#
                ),
                &mut state,
            )
            .unwrap();
            assert_eq!(
                events,
                vec![StreamEvent::Completed {
                    finish_reason: expected
                }]
            );
        }

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

    #[test]
    fn function_call_stream_across_events() {
        let mut state = EventDecodeState::default();
        let mut events = Vec::new();
        events.extend(
            decode_sse_data(
                r#"{"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","call_id":"call_1","name":"lookup","arguments":""}}"#,
                &mut state,
            )
            .unwrap(),
        );
        events.extend(
            decode_sse_data(
                r#"{"type":"response.function_call_arguments.delta","output_index":0,"delta":"{\"q\":"}"#,
                &mut state,
            )
            .unwrap(),
        );
        events.extend(
            decode_sse_data(
                r#"{"type":"response.function_call_arguments.delta","output_index":0,"delta":"1}"}"#,
                &mut state,
            )
            .unwrap(),
        );
        events.extend(
            decode_sse_data(
                r#"{"type":"response.function_call_arguments.done","output_index":0,"arguments":"{\"q\":1}"}"#,
                &mut state,
            )
            .unwrap(),
        );
        events.extend(
            decode_sse_data(
                r#"{"type":"response.completed","response":{"status":"completed","output":[{"type":"function_call","call_id":"call_1","name":"lookup","arguments":"{\"q\":1}"}]}}"#,
                &mut state,
            )
            .unwrap(),
        );
        assert!(events.iter().any(|e| matches!(
            e,
            StreamEvent::ToolCallFinished {
                id,
                name,
                arguments,
                ..
            } if id == "call_1" && name == "lookup" && arguments == r#"{"q":1}"#
        )));
        assert!(matches!(
            events.last(),
            Some(StreamEvent::Completed {
                finish_reason: FinishReason::ToolCalls
            })
        ));
    }
}
