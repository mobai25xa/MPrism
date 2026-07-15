//! Decode OpenAI `chat.completion.chunk` SSE data payloads into StreamEvent.

use crate::error::{parse_provider_error_body, ProtocolError, ProtocolErrorKind};
use crate::finish::openai_chat_completions as map_finish;
use crate::types::StreamEvent;
use crate::usage::from_openai_chat_usage;
use serde_json::Value;
use std::collections::HashMap;

/// Accumulator for one in-flight tool call across chunks.
#[derive(Debug, Default)]
struct PendingToolCall {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

/// Per-stream decoder state for OpenAI chat chunks.
#[derive(Debug, Default)]
pub struct ChunkDecodeState {
    pub finish_reason: Option<String>,
    pub completed: bool,
    pending_tools: HashMap<u32, PendingToolCall>,
}

/// Decode one SSE `data` payload into zero or more stream events.
pub fn decode_sse_data(
    data: &str,
    state: &mut ChunkDecodeState,
) -> Result<Vec<StreamEvent>, ProtocolError> {
    let trimmed = data.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    if trimmed == "[DONE]" {
        if state.completed {
            return Ok(Vec::new());
        }
        state.completed = true;
        let mut events = finish_pending_tools(state);
        events.push(StreamEvent::Completed {
            finish_reason: map_finish(state.finish_reason.as_deref()),
        });
        return Ok(events);
    }

    let value: Value = serde_json::from_str(trimmed).map_err(|err| {
        ProtocolError::new(
            ProtocolErrorKind::Decode,
            format!("SSE JSON 解码失败: {err}"),
        )
    })?;

    if value.get("error").is_some() {
        let (msg, code) = parse_provider_error_body(trimmed);
        let mut err = ProtocolError::new(
            ProtocolErrorKind::ProviderUnavailable,
            msg.unwrap_or_else(|| "服务商在流中返回错误".into()),
        );
        if let Some(code) = code {
            err = err.with_provider_code(code);
        }
        return Err(err);
    }

    let mut events = Vec::new();

    if let Some(usage_val) = value.get("usage") {
        if let Some(usage) = from_openai_chat_usage(usage_val) {
            events.push(StreamEvent::Usage(usage));
        }
    }

    // Only consume choices[0] (mapping-v2 §8).
    if let Some(choice) = value
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
    {
        if let Some(delta) = choice.get("delta") {
            if let Some(text) = non_empty_str(
                delta
                    .get("reasoning_content")
                    .or_else(|| delta.get("reasoning")),
            ) {
                events.push(StreamEvent::ReasoningDelta { text });
            }
            if let Some(text) = non_empty_str(delta.get("content")) {
                events.push(StreamEvent::ContentDelta { text });
            }
            if let Some(tool_calls) = delta.get("tool_calls").and_then(|t| t.as_array()) {
                for tc in tool_calls {
                    events.extend(decode_tool_call_delta(state, tc));
                }
            }
        }
        if let Some(reason) = choice
            .get("finish_reason")
            .and_then(|r| r.as_str())
            .filter(|s| !s.is_empty())
        {
            state.finish_reason = Some(reason.to_string());
            // Official streams often set finish_reason on the chunk that ends tool_calls.
            if reason == "tool_calls" {
                events.extend(finish_pending_tools(state));
            }
        }
    }

    Ok(events)
}

fn decode_tool_call_delta(state: &mut ChunkDecodeState, tc: &Value) -> Vec<StreamEvent> {
    let index = tc
        .get("index")
        .and_then(|v| v.as_u64())
        .map(|n| n as u32)
        .unwrap_or(0);
    let entry = state.pending_tools.entry(index).or_default();
    if let Some(id) = tc
        .get("id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        entry.id = Some(id.to_string());
    }
    if let Some(name) = tc
        .get("function")
        .and_then(|f| f.get("name"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        entry.name = Some(name.to_string());
    }
    let args_delta = tc
        .get("function")
        .and_then(|f| f.get("arguments"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if !args_delta.is_empty() {
        entry.arguments.push_str(args_delta);
    }
    // Emit delta even when only id/name arrive (arguments_delta may be empty).
    let has_any = entry.id.is_some()
        || entry.name.is_some()
        || !args_delta.is_empty()
        || tc.get("function").is_some();
    if !has_any {
        return Vec::new();
    }
    vec![StreamEvent::ToolCallDelta {
        id: entry.id.clone(),
        name: entry.name.clone(),
        arguments_delta: args_delta.to_string(),
        index: Some(index),
    }]
}

fn finish_pending_tools(state: &mut ChunkDecodeState) -> Vec<StreamEvent> {
    if state.pending_tools.is_empty() {
        return Vec::new();
    }
    let mut indices: Vec<u32> = state.pending_tools.keys().copied().collect();
    indices.sort_unstable();
    let mut events = Vec::new();
    for index in indices {
        if let Some(pending) = state.pending_tools.remove(&index) {
            let id = pending.id.unwrap_or_default();
            let name = pending.name.unwrap_or_default();
            // Skip incomplete empties that never received data.
            if id.is_empty() && name.is_empty() && pending.arguments.is_empty() {
                continue;
            }
            events.push(StreamEvent::ToolCallFinished {
                id,
                name,
                arguments: pending.arguments,
                index: Some(index),
            });
        }
    }
    events
}

fn non_empty_str(value: Option<&Value>) -> Option<String> {
    value
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{FinishReason, TokenUsage};

    #[test]
    fn content_reasoning_usage_done() {
        let mut state = ChunkDecodeState::default();
        let mut events = Vec::new();
        events.extend(
            decode_sse_data(
                r#"{"choices":[{"delta":{"reasoning_content":"think","content":"Hel"}}]}"#,
                &mut state,
            )
            .unwrap(),
        );
        events.extend(
            decode_sse_data(
                r#"{"choices":[{"delta":{"content":"lo"},"finish_reason":"stop"}]}"#,
                &mut state,
            )
            .unwrap(),
        );
        events.extend(
            decode_sse_data(
                r#"{"choices":[],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3,"completion_tokens_details":{"reasoning_tokens":9},"prompt_tokens_details":{"cached_tokens":1}}}"#,
                &mut state,
            )
            .unwrap(),
        );
        events.extend(decode_sse_data("[DONE]", &mut state).unwrap());

        assert!(matches!(events[0], StreamEvent::ReasoningDelta { ref text } if text == "think"));
        assert!(matches!(events[1], StreamEvent::ContentDelta { ref text } if text == "Hel"));
        assert!(matches!(events[2], StreamEvent::ContentDelta { ref text } if text == "lo"));
        assert!(matches!(
            events[3],
            StreamEvent::Usage(TokenUsage {
                total_tokens: Some(3),
                reasoning_tokens: Some(9),
                cached_tokens: Some(1),
                ..
            })
        ));
        assert!(matches!(
            events[4],
            StreamEvent::Completed {
                finish_reason: FinishReason::Stop
            }
        ));
        assert!(state.completed);
        assert!(decode_sse_data("[DONE]", &mut state).unwrap().is_empty());
    }

    #[test]
    fn tool_calls_stream_across_chunks() {
        let mut state = ChunkDecodeState::default();
        let mut events = Vec::new();
        events.extend(
            decode_sse_data(
                r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"lookup","arguments":""}}]}}]}"#,
                &mut state,
            )
            .unwrap(),
        );
        events.extend(
            decode_sse_data(
                r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"q\":"}}]}}]}"#,
                &mut state,
            )
            .unwrap(),
        );
        events.extend(
            decode_sse_data(
                r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"1}"}}]},"finish_reason":"tool_calls"}]}"#,
                &mut state,
            )
            .unwrap(),
        );
        events.extend(decode_sse_data("[DONE]", &mut state).unwrap());

        assert!(events.iter().any(|e| matches!(
            e,
            StreamEvent::ToolCallDelta {
                id: Some(id),
                name: Some(name),
                ..
            } if id == "call_1" && name == "lookup"
        )));
        assert!(events.iter().any(|e| matches!(
            e,
            StreamEvent::ToolCallFinished {
                id,
                name,
                arguments,
                index: Some(0)
            } if id == "call_1" && name == "lookup" && arguments == r#"{"q":1}"#
        )));
        assert!(matches!(
            events.last(),
            Some(StreamEvent::Completed {
                finish_reason: FinishReason::ToolCalls
            })
        ));
    }

    #[test]
    fn finish_reason_length_and_tool_calls() {
        for (raw, expected) in [
            ("length", FinishReason::Length),
            ("content_filter", FinishReason::ContentFilter),
            ("tool_calls", FinishReason::ToolCalls),
        ] {
            let mut state = ChunkDecodeState::default();
            decode_sse_data(
                &format!(r#"{{"choices":[{{"delta":{{}},"finish_reason":"{raw}"}}]}}"#),
                &mut state,
            )
            .unwrap();
            let events = decode_sse_data("[DONE]", &mut state).unwrap();
            assert_eq!(
                events,
                vec![StreamEvent::Completed {
                    finish_reason: expected
                }]
            );
        }
    }

    #[test]
    fn only_first_choice_and_single_completed() {
        let mut state = ChunkDecodeState::default();
        let events = decode_sse_data(
            r#"{"choices":[{"delta":{"content":"A"}},{"delta":{"content":"B"}}]}"#,
            &mut state,
        )
        .unwrap();
        assert_eq!(events, vec![StreamEvent::ContentDelta { text: "A".into() }]);
        state.finish_reason = Some("stop".into());
        let first = decode_sse_data("[DONE]", &mut state).unwrap();
        assert_eq!(first.len(), 1);
        let second = decode_sse_data("[DONE]", &mut state).unwrap();
        assert!(second.is_empty());
    }

    #[test]
    fn reasoning_alias_and_malformed() {
        let mut state = ChunkDecodeState::default();
        let events =
            decode_sse_data(r#"{"choices":[{"delta":{"reasoning":"r2"}}]}"#, &mut state).unwrap();
        assert!(matches!(events[0], StreamEvent::ReasoningDelta { ref text } if text == "r2"));
        let err = decode_sse_data("{not-json", &mut state).unwrap_err();
        assert_eq!(err.kind, ProtocolErrorKind::Decode);
    }
}
