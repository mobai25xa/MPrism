//! Decode Gemini streamGenerateContent SSE JSON into StreamEvent.

use crate::error::{redact_secrets, ProtocolError, ProtocolErrorKind};
use crate::finish::gemini_generate_content as map_finish;
use crate::types::StreamEvent;
use crate::usage::from_gemini_usage_metadata;
use serde_json::Value;

/// Per-stream decoder state for Gemini SSE frames.
#[derive(Debug, Default)]
pub struct EventDecodeState {
    pub completed: bool,
    usage_emitted: bool,
}

/// Decode one SSE `data` payload (full GenerateContentResponse JSON).
pub fn decode_sse_data(
    data: &str,
    state: &mut EventDecodeState,
) -> Result<Vec<StreamEvent>, ProtocolError> {
    let trimmed = data.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    // Tolerate proxy-injected [DONE].
    if trimmed == "[DONE]" {
        if state.completed {
            return Ok(Vec::new());
        }
        state.completed = true;
        return Ok(vec![StreamEvent::Completed {
            finish_reason: map_finish(Some("STOP")),
        }]);
    }

    let value: Value = serde_json::from_str(trimmed).map_err(|err| {
        ProtocolError::new(
            ProtocolErrorKind::Decode,
            format!("SSE JSON 解码失败: {err}"),
        )
    })?;

    if value.get("error").is_some() {
        return Err(error_from_json(&value));
    }

    if state.completed {
        return Ok(Vec::new());
    }

    // Prompt blocked with no candidates.
    if let Some(feedback) = value.get("promptFeedback") {
        let blocked = feedback
            .get("blockReason")
            .and_then(|b| b.as_str())
            .filter(|s| !s.is_empty());
        let has_candidates = value
            .get("candidates")
            .and_then(|c| c.as_array())
            .map(|a| !a.is_empty())
            .unwrap_or(false);
        if let Some(reason) = blocked {
            if !has_candidates {
                return Err(ProtocolError::new(
                    ProtocolErrorKind::InvalidRequest,
                    format!("提示被拦截: {reason}"),
                ));
            }
        }
    }

    let mut events = Vec::new();

    if let Some(candidates) = value.get("candidates").and_then(|c| c.as_array()) {
        if let Some(candidate) = candidates.first() {
            if let Some(parts) = candidate
                .get("content")
                .and_then(|c| c.get("parts"))
                .and_then(|p| p.as_array())
            {
                for (part_index, part) in parts.iter().enumerate() {
                    if let Some(fc) = part.get("functionCall") {
                        let name = fc
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let args = fc
                            .get("args")
                            .cloned()
                            .unwrap_or(Value::Object(Default::default()));
                        let arguments =
                            serde_json::to_string(&args).unwrap_or_else(|_| "{}".into());
                        // Gemini often delivers the full functionCall in one part (not token-streamed).
                        let index = Some(part_index as u32);
                        let id = name.clone();
                        events.push(StreamEvent::ToolCallDelta {
                            id: Some(id.clone()),
                            name: Some(name.clone()),
                            arguments_delta: arguments.clone(),
                            index,
                        });
                        events.push(StreamEvent::ToolCallFinished {
                            id,
                            name,
                            arguments,
                            index,
                        });
                        continue;
                    }
                    let thought = part
                        .get("thought")
                        .and_then(|t| t.as_bool())
                        .unwrap_or(false);
                    let Some(text) = part
                        .get("text")
                        .and_then(|t| t.as_str())
                        .filter(|s| !s.is_empty())
                    else {
                        continue;
                    };
                    if thought {
                        events.push(StreamEvent::ReasoningDelta {
                            text: text.to_string(),
                        });
                    } else {
                        events.push(StreamEvent::ContentDelta {
                            text: text.to_string(),
                        });
                    }
                }
            }

            if let Some(reason) = candidate
                .get("finishReason")
                .and_then(|r| r.as_str())
                .filter(|s| !s.is_empty())
            {
                if !state.usage_emitted {
                    if let Some(usage_val) = value.get("usageMetadata") {
                        if let Some(usage) = from_gemini_usage_metadata(usage_val) {
                            state.usage_emitted = true;
                            events.push(StreamEvent::Usage(usage));
                        }
                    }
                }
                state.completed = true;
                events.push(StreamEvent::Completed {
                    finish_reason: map_finish(Some(reason)),
                });
                return Ok(events);
            }
        }
    }

    // Early usage-only frames are deferred until finishReason to avoid double emit.
    Ok(events)
}

fn error_from_json(value: &Value) -> ProtocolError {
    let err = value.get("error");
    let message = err
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
        .unwrap_or("服务商返回错误");
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
        });
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
    fn text_and_thought_deltas() {
        let mut state = EventDecodeState::default();
        let events = decode_sse_data(
            r#"{"candidates":[{"content":{"parts":[{"text":"Hi"},{"text":"plan","thought":true}]},"index":0}]}"#,
            &mut state,
        )
        .unwrap();
        assert_eq!(
            events,
            vec![
                StreamEvent::ContentDelta { text: "Hi".into() },
                StreamEvent::ReasoningDelta {
                    text: "plan".into()
                },
            ]
        );
    }

    #[test]
    fn function_call_part_emits_tool_events() {
        let mut state = EventDecodeState::default();
        let events = decode_sse_data(
            r#"{"candidates":[{"content":{"parts":[{"functionCall":{"name":"lookup","args":{"q":1}}}]},"finishReason":"FUNCTION_CALL"}]}"#,
            &mut state,
        )
        .unwrap();
        assert!(events.iter().any(|e| matches!(
            e,
            StreamEvent::ToolCallFinished {
                name,
                arguments,
                ..
            } if name == "lookup" && arguments.contains("\"q\":1")
        )));
        assert!(matches!(
            events.last(),
            Some(StreamEvent::Completed {
                finish_reason: FinishReason::ToolCalls
            })
        ));
    }

    #[test]
    fn finish_emits_usage_and_completed() {
        let mut state = EventDecodeState::default();
        let events = decode_sse_data(
            r#"{"candidates":[{"content":{"parts":[{"text":"lo"}]},"finishReason":"STOP","index":0}],"usageMetadata":{"promptTokenCount":3,"candidatesTokenCount":2,"totalTokenCount":5,"thoughtsTokenCount":7}}"#,
            &mut state,
        )
        .unwrap();
        assert_eq!(events.len(), 3);
        match &events[0] {
            StreamEvent::ContentDelta { text } => assert_eq!(text, "lo"),
            other => panic!("expected content, got {other:?}"),
        }
        match &events[1] {
            StreamEvent::Usage(u) => {
                assert_eq!(u.prompt_tokens, Some(3));
                assert_eq!(u.completion_tokens, Some(2));
                assert_eq!(u.total_tokens, Some(5));
                assert_eq!(u.reasoning_tokens, Some(7));
            }
            other => panic!("expected usage, got {other:?}"),
        }
        match &events[2] {
            StreamEvent::Completed { finish_reason } => {
                assert_eq!(finish_reason, &FinishReason::Stop);
            }
            other => panic!("expected completed, got {other:?}"),
        }
        assert!(state.completed);
    }

    #[test]
    fn finish_reason_table() {
        for (reason, expected) in [
            ("MAX_TOKENS", FinishReason::Length),
            ("SAFETY", FinishReason::ContentFilter),
            ("FUNCTION_CALL", FinishReason::ToolCalls),
            ("RECITATION", FinishReason::Other("RECITATION".into())),
        ] {
            let mut state = EventDecodeState::default();
            let events = decode_sse_data(
                &format!(
                    r#"{{"candidates":[{{"content":{{"parts":[]}},"finishReason":"{reason}"}}]}}"#
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
    }

    #[test]
    fn prompt_block_without_candidates() {
        let mut state = EventDecodeState::default();
        let err = decode_sse_data(r#"{"promptFeedback":{"blockReason":"SAFETY"}}"#, &mut state)
            .unwrap_err();
        assert_eq!(err.kind, ProtocolErrorKind::InvalidRequest);
    }

    #[test]
    fn stream_error_payload() {
        let mut state = EventDecodeState::default();
        let err = decode_sse_data(
            r#"{"error":{"code":503,"message":"busy AIza-leaked","status":"UNAVAILABLE"}}"#,
            &mut state,
        )
        .unwrap_err();
        assert_eq!(err.provider_code.as_deref(), Some("UNAVAILABLE"));
    }
}
