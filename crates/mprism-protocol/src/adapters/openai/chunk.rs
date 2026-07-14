//! Decode OpenAI `chat.completion.chunk` SSE data payloads into StreamEvent.

use crate::error::{parse_provider_error_body, ProtocolError, ProtocolErrorKind};
use crate::types::{StreamEvent, TokenUsage};
use serde::Deserialize;
use serde_json::Value;

/// Per-stream decoder state for OpenAI chat chunks.
#[derive(Debug, Default)]
pub struct ChunkDecodeState {
    pub finish_reason: Option<String>,
    pub completed: bool,
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
        return Ok(vec![StreamEvent::Completed {
            finish_reason: state.finish_reason.clone(),
        }]);
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

    let chunk: ChatCompletionChunk = serde_json::from_value(value).map_err(|err| {
        ProtocolError::new(
            ProtocolErrorKind::Decode,
            format!("chat.completion.chunk 解码失败: {err}"),
        )
    })?;

    let mut events = Vec::new();

    if let Some(usage) = chunk.usage {
        events.push(StreamEvent::Usage(TokenUsage {
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            total_tokens: usage.total_tokens,
        }));
    }

    if let Some(choice) = chunk.choices.into_iter().next() {
        if let Some(delta) = choice.delta {
            if let Some(text) =
                non_empty(delta.reasoning_content).or_else(|| non_empty(delta.reasoning))
            {
                events.push(StreamEvent::ReasoningDelta { text });
            }
            if let Some(text) = non_empty(delta.content) {
                events.push(StreamEvent::ContentDelta { text });
            }
        }
        if let Some(reason) = choice.finish_reason {
            if !reason.is_empty() {
                state.finish_reason = Some(reason);
            }
        }
    }

    Ok(events)
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.and_then(|s| if s.is_empty() { None } else { Some(s) })
}

#[derive(Debug, Deserialize)]
struct ChatCompletionChunk {
    #[serde(default)]
    choices: Vec<ChunkChoice>,
    #[serde(default)]
    usage: Option<WireUsage>,
}

#[derive(Debug, Deserialize)]
struct ChunkChoice {
    #[serde(default)]
    delta: Option<ChunkDelta>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct ChunkDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    reasoning: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WireUsage {
    #[serde(default)]
    prompt_tokens: Option<u32>,
    #[serde(default)]
    completion_tokens: Option<u32>,
    #[serde(default)]
    total_tokens: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

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
                r#"{"choices":[],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}}"#,
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
                ..
            })
        ));
        assert!(matches!(
            events[4],
            StreamEvent::Completed {
                finish_reason: Some(ref r)
            } if r == "stop"
        ));
        assert!(state.completed);
        assert!(decode_sse_data("[DONE]", &mut state).unwrap().is_empty());
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
