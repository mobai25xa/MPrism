//! Protocol-specific finish-reason mappers (mapping-v2 tables).
//!
//! Public streams only expose [`FinishReason`]; raw provider strings are not part of the API.

use crate::types::FinishReason;

/// OpenAI Chat Completions `choices[].finish_reason` (mapping-v2 §8).
pub fn openai_chat_completions(raw: Option<&str>) -> FinishReason {
    match normalize(raw) {
        None => FinishReason::Other("unknown".into()),
        Some("stop") => FinishReason::Stop,
        Some("length") => FinishReason::Length,
        Some("content_filter") => FinishReason::ContentFilter,
        Some("tool_calls") | Some("function_call") => FinishReason::ToolCalls,
        Some(other) => FinishReason::Other(other.to_string()),
    }
}

/// OpenAI Responses terminal status / incomplete reason (mapping-v2 §6).
pub fn openai_responses(raw: Option<&str>) -> FinishReason {
    match normalize(raw) {
        None => FinishReason::Other("unknown".into()),
        Some("stop") | Some("completed") => FinishReason::Stop,
        Some("max_output_tokens") | Some("length") => FinishReason::Length,
        Some("content_filter") | Some("content_filter_policy") => FinishReason::ContentFilter,
        Some("tool_calls") | Some("function_call") | Some("tool_use") | Some("required_tool") => {
            FinishReason::ToolCalls
        }
        Some(other) => FinishReason::Other(other.to_string()),
    }
}

/// Anthropic Messages `stop_reason` (mapping-v2 §6).
pub fn anthropic_messages(raw: Option<&str>) -> FinishReason {
    match normalize(raw) {
        None => FinishReason::Stop, // message_stop without reason ≈ end_turn
        Some("end_turn") | Some("stop_sequence") => FinishReason::Stop,
        Some("max_tokens") => FinishReason::Length,
        Some("tool_use") => FinishReason::ToolCalls,
        Some("refusal") => FinishReason::ContentFilter,
        Some(other) => FinishReason::Other(other.to_string()),
    }
}

/// Gemini `candidates[].finishReason` (mapping-v2 §7); comparison is case-insensitive.
pub fn gemini_generate_content(raw: Option<&str>) -> FinishReason {
    let Some(raw) = normalize(raw) else {
        return FinishReason::Other("unknown".into());
    };
    match raw.to_ascii_uppercase().as_str() {
        "STOP" => FinishReason::Stop,
        "MAX_TOKENS" => FinishReason::Length,
        "SAFETY" | "BLOCKLIST" | "PROHIBITED_CONTENT" | "SPII" | "IMAGE_SAFETY" => {
            FinishReason::ContentFilter
        }
        // Tool-related finish values when present on the wire.
        "TOOL_CALL" | "TOOL_CALLS" | "FUNCTION_CALL" | "MALFORMED_FUNCTION_CALL" => {
            FinishReason::ToolCalls
        }
        other => FinishReason::Other(other.to_string()),
    }
}

fn normalize(raw: Option<&str>) -> Option<&str> {
    raw.map(str::trim).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_chat_completions_table() {
        assert_eq!(openai_chat_completions(Some("stop")), FinishReason::Stop);
        assert_eq!(
            openai_chat_completions(Some("length")),
            FinishReason::Length
        );
        assert_eq!(
            openai_chat_completions(Some("content_filter")),
            FinishReason::ContentFilter
        );
        assert_eq!(
            openai_chat_completions(Some("tool_calls")),
            FinishReason::ToolCalls
        );
        assert_eq!(
            openai_chat_completions(Some("function_call")),
            FinishReason::ToolCalls
        );
        assert_eq!(
            openai_chat_completions(Some("weird")),
            FinishReason::Other("weird".into())
        );
        assert_eq!(
            openai_chat_completions(None),
            FinishReason::Other("unknown".into())
        );
    }

    #[test]
    fn openai_responses_table() {
        assert_eq!(openai_responses(Some("completed")), FinishReason::Stop);
        assert_eq!(openai_responses(Some("stop")), FinishReason::Stop);
        assert_eq!(
            openai_responses(Some("max_output_tokens")),
            FinishReason::Length
        );
        assert_eq!(
            openai_responses(Some("content_filter")),
            FinishReason::ContentFilter
        );
        assert_eq!(openai_responses(Some("tool_use")), FinishReason::ToolCalls);
        assert_eq!(
            openai_responses(Some("custom_incomplete")),
            FinishReason::Other("custom_incomplete".into())
        );
    }

    #[test]
    fn anthropic_messages_table() {
        assert_eq!(anthropic_messages(Some("end_turn")), FinishReason::Stop);
        assert_eq!(
            anthropic_messages(Some("stop_sequence")),
            FinishReason::Stop
        );
        assert_eq!(anthropic_messages(Some("max_tokens")), FinishReason::Length);
        assert_eq!(
            anthropic_messages(Some("tool_use")),
            FinishReason::ToolCalls
        );
        assert_eq!(
            anthropic_messages(Some("refusal")),
            FinishReason::ContentFilter
        );
        assert_eq!(anthropic_messages(None), FinishReason::Stop);
        assert_eq!(
            anthropic_messages(Some("other_reason")),
            FinishReason::Other("other_reason".into())
        );
    }

    #[test]
    fn gemini_generate_content_table() {
        assert_eq!(gemini_generate_content(Some("STOP")), FinishReason::Stop);
        assert_eq!(gemini_generate_content(Some("stop")), FinishReason::Stop);
        assert_eq!(
            gemini_generate_content(Some("MAX_TOKENS")),
            FinishReason::Length
        );
        assert_eq!(
            gemini_generate_content(Some("SAFETY")),
            FinishReason::ContentFilter
        );
        assert_eq!(
            gemini_generate_content(Some("PROHIBITED_CONTENT")),
            FinishReason::ContentFilter
        );
        assert_eq!(
            gemini_generate_content(Some("FUNCTION_CALL")),
            FinishReason::ToolCalls
        );
        assert_eq!(
            gemini_generate_content(Some("RECITATION")),
            FinishReason::Other("RECITATION".into())
        );
    }
}
