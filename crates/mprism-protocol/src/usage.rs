//! Token usage extraction helpers from provider wire JSON.

use crate::types::TokenUsage;
use serde_json::Value;

/// OpenAI Chat Completions `usage` object.
pub fn from_openai_chat_usage(usage: &Value) -> Option<TokenUsage> {
    let prompt = as_u32(usage.get("prompt_tokens"));
    let completion = as_u32(usage.get("completion_tokens"));
    let total = as_u32(usage.get("total_tokens"));
    let reasoning = usage
        .get("completion_tokens_details")
        .and_then(|d| as_u32(d.get("reasoning_tokens")));
    let cached = usage
        .get("prompt_tokens_details")
        .and_then(|d| as_u32(d.get("cached_tokens")));
    if prompt.is_none()
        && completion.is_none()
        && total.is_none()
        && reasoning.is_none()
        && cached.is_none()
    {
        return None;
    }
    Some(TokenUsage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: total,
        reasoning_tokens: reasoning,
        cached_tokens: cached,
    })
}

/// OpenAI Responses `response.usage` object.
pub fn from_openai_responses_usage(usage: &Value) -> Option<TokenUsage> {
    let prompt = as_u32(usage.get("input_tokens")).or_else(|| as_u32(usage.get("prompt_tokens")));
    let completion =
        as_u32(usage.get("output_tokens")).or_else(|| as_u32(usage.get("completion_tokens")));
    let total = as_u32(usage.get("total_tokens"));
    let reasoning = usage
        .get("output_tokens_details")
        .and_then(|d| as_u32(d.get("reasoning_tokens")));
    let cached = usage
        .get("input_tokens_details")
        .and_then(|d| as_u32(d.get("cached_tokens")));
    if prompt.is_none()
        && completion.is_none()
        && total.is_none()
        && reasoning.is_none()
        && cached.is_none()
    {
        return None;
    }
    Some(TokenUsage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: total,
        reasoning_tokens: reasoning,
        cached_tokens: cached,
    })
}

/// Anthropic Messages usage fields (`input_tokens` / `output_tokens` + optional details).
pub fn from_anthropic_usage(
    input_tokens: Option<u32>,
    output_tokens: Option<u32>,
    usage: Option<&Value>,
) -> Option<TokenUsage> {
    let mut input = input_tokens;
    let mut output = output_tokens;
    let mut reasoning = None;
    let mut cached = None;
    if let Some(usage) = usage {
        if let Some(n) = as_u32(usage.get("input_tokens")) {
            input = Some(n);
        }
        if let Some(n) = as_u32(usage.get("output_tokens")) {
            output = Some(n);
        }
        reasoning = usage
            .get("output_tokens_details")
            .and_then(|d| as_u32(d.get("thinking_tokens")));
        // Common cache field names when present.
        cached = as_u32(usage.get("cache_read_input_tokens"))
            .or_else(|| as_u32(usage.get("cache_creation_input_tokens")));
    }
    if input.is_none() && output.is_none() && reasoning.is_none() && cached.is_none() {
        return None;
    }
    let total = match (input, output) {
        (Some(i), Some(o)) => Some(i.saturating_add(o)),
        _ => None,
    };
    Some(TokenUsage {
        prompt_tokens: input,
        completion_tokens: output,
        total_tokens: total,
        reasoning_tokens: reasoning,
        cached_tokens: cached,
    })
}

/// Gemini `usageMetadata` object (camelCase wire).
pub fn from_gemini_usage_metadata(usage: &Value) -> Option<TokenUsage> {
    let prompt =
        as_u32(usage.get("promptTokenCount")).or_else(|| as_u32(usage.get("prompt_token_count")));
    let completion = as_u32(usage.get("candidatesTokenCount"))
        .or_else(|| as_u32(usage.get("candidates_token_count")));
    let total =
        as_u32(usage.get("totalTokenCount")).or_else(|| as_u32(usage.get("total_token_count")));
    let reasoning = as_u32(usage.get("thoughtsTokenCount"))
        .or_else(|| as_u32(usage.get("thoughts_token_count")));
    let cached = as_u32(usage.get("cachedContentTokenCount"))
        .or_else(|| as_u32(usage.get("cached_content_token_count")));
    if prompt.is_none()
        && completion.is_none()
        && total.is_none()
        && reasoning.is_none()
        && cached.is_none()
    {
        return None;
    }
    Some(TokenUsage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: total,
        reasoning_tokens: reasoning,
        cached_tokens: cached,
    })
}

fn as_u32(value: Option<&Value>) -> Option<u32> {
    value.and_then(|v| v.as_u64()).map(|n| n as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn openai_chat_usage_with_details() {
        let u = from_openai_chat_usage(&json!({
            "prompt_tokens": 10,
            "completion_tokens": 5,
            "total_tokens": 15,
            "prompt_tokens_details": { "cached_tokens": 4 },
            "completion_tokens_details": { "reasoning_tokens": 3 }
        }))
        .unwrap();
        assert_eq!(u.prompt_tokens, Some(10));
        assert_eq!(u.completion_tokens, Some(5));
        assert_eq!(u.total_tokens, Some(15));
        assert_eq!(u.reasoning_tokens, Some(3));
        assert_eq!(u.cached_tokens, Some(4));
    }

    #[test]
    fn openai_responses_usage_with_details() {
        let u = from_openai_responses_usage(&json!({
            "input_tokens": 8,
            "output_tokens": 6,
            "total_tokens": 14,
            "output_tokens_details": { "reasoning_tokens": 2 },
            "input_tokens_details": { "cached_tokens": 1 }
        }))
        .unwrap();
        assert_eq!(u.prompt_tokens, Some(8));
        assert_eq!(u.completion_tokens, Some(6));
        assert_eq!(u.reasoning_tokens, Some(2));
        assert_eq!(u.cached_tokens, Some(1));
    }

    #[test]
    fn anthropic_usage_thinking_tokens() {
        let u = from_anthropic_usage(
            Some(3),
            Some(2),
            Some(&json!({
                "output_tokens": 2,
                "output_tokens_details": { "thinking_tokens": 9 },
                "cache_read_input_tokens": 1
            })),
        )
        .unwrap();
        assert_eq!(u.total_tokens, Some(5));
        assert_eq!(u.reasoning_tokens, Some(9));
        assert_eq!(u.cached_tokens, Some(1));
    }

    #[test]
    fn gemini_usage_thoughts() {
        let u = from_gemini_usage_metadata(&json!({
            "promptTokenCount": 3,
            "candidatesTokenCount": 2,
            "totalTokenCount": 5,
            "thoughtsTokenCount": 7,
            "cachedContentTokenCount": 1
        }))
        .unwrap();
        assert_eq!(u.reasoning_tokens, Some(7));
        assert_eq!(u.cached_tokens, Some(1));
    }
}
