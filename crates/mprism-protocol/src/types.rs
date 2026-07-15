//! Public protocol types for MPrism model adapters.
//!
//! Field semantics follow `docs/sources/model-protocol-sdk.md` §3.

use crate::error::{ProtocolError, ProtocolErrorKind};
use crate::secret::SecretString;
use serde_json::Value;
use url::Url;

/// Max decoded size for inline base64 images (4 MiB).
pub const MAX_INLINE_IMAGE_BYTES: usize = 4 * 1024 * 1024;

/// Supported wire protocols.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProtocolKind {
    /// OpenAI Chat Completions (`POST /chat/completions`).
    OpenAiChatCompletions,
    /// OpenAI Responses API (`POST /responses`).
    OpenAiResponses,
    /// Anthropic Messages API (`POST /messages`).
    AnthropicMessages,
    /// Gemini generateContent / streamGenerateContent.
    GeminiGenerateContent,
}

/// Optional auth extensions (default empty = V1 behavior).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthOptions {
    pub extra_headers: Vec<(String, String)>,
    pub api_key_query_param: Option<String>,
}

/// Provider connection settings used by adapters.
#[derive(Debug, Clone)]
pub struct ProviderEndpoint {
    pub protocol: ProtocolKind,
    pub base_url: Url,
    pub api_key: SecretString,
    pub auth: AuthOptions,
}

impl ProviderEndpoint {
    /// Build an endpoint after validating base URL shape.
    pub fn new(
        protocol: ProtocolKind,
        base_url: impl AsRef<str>,
        api_key: impl Into<SecretString>,
    ) -> Result<Self, ProtocolError> {
        let base_url = normalize_base_url(base_url.as_ref())?;
        Ok(Self {
            protocol,
            base_url,
            api_key: api_key.into(),
            auth: AuthOptions::default(),
        })
    }
}

/// Normalize provider base URL: no query/fragment, strip trailing slash, keep path.
pub fn normalize_base_url(raw: &str) -> Result<Url, ProtocolError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ProtocolError::new(
            ProtocolErrorKind::InvalidConfiguration,
            "Base URL 不能为空",
        ));
    }

    let mut url = Url::parse(trimmed).map_err(|err| {
        ProtocolError::new(
            ProtocolErrorKind::InvalidConfiguration,
            format!("Base URL 无效: {err}"),
        )
    })?;

    match url.scheme() {
        "http" | "https" => {}
        other => {
            return Err(ProtocolError::new(
                ProtocolErrorKind::InvalidConfiguration,
                format!("Base URL 仅支持 http/https，收到: {other}"),
            ));
        }
    }

    if url.query().is_some() || url.fragment().is_some() {
        return Err(ProtocolError::new(
            ProtocolErrorKind::InvalidConfiguration,
            "Base URL 不能包含 query 或 fragment",
        ));
    }

    let path = url.path().to_string();
    if path.len() > 1 && path.ends_with('/') {
        url.set_path(path.trim_end_matches('/'));
    }

    Ok(url)
}

/// Join `segment` onto base path while preserving existing path segments.
pub fn join_api_path(base: &Url, segment: &str) -> Result<Url, ProtocolError> {
    let segment = segment.trim_matches('/');
    if segment.is_empty() {
        return Err(ProtocolError::new(
            ProtocolErrorKind::InvalidConfiguration,
            "API 路径段不能为空",
        ));
    }

    let mut url = base.clone();
    let mut path = url.path().trim_end_matches('/').to_string();
    if path.is_empty() {
        path = String::from("/");
    }
    if path == "/" {
        path = format!("/{segment}");
    } else {
        path = format!("{path}/{segment}");
    }
    url.set_path(&path);
    Ok(url)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelInfo {
    pub id: String,
    pub display_name: String,
    pub owned_by: Option<String>,
}

/// Message content part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentPart {
    Text { text: String },
    ImageUrl { url: String, detail: Option<String> },
    ImageBase64 { media_type: String, data: String },
}

impl ContentPart {
    fn validate(&self) -> Result<(), ProtocolError> {
        match self {
            Self::Text { .. } => Ok(()),
            Self::ImageUrl { url, .. } => {
                if url.trim().is_empty() {
                    return Err(ProtocolError::new(
                        ProtocolErrorKind::InvalidRequest,
                        "ImageUrl.url 不能为空",
                    ));
                }
                Ok(())
            }
            Self::ImageBase64 { media_type, data } => {
                if media_type.trim().is_empty() {
                    return Err(ProtocolError::new(
                        ProtocolErrorKind::InvalidRequest,
                        "ImageBase64.media_type 不能为空",
                    ));
                }
                if data.trim().is_empty() {
                    return Err(ProtocolError::new(
                        ProtocolErrorKind::InvalidRequest,
                        "ImageBase64.data 不能为空",
                    ));
                }
                let decoded_len = approx_base64_decoded_len(data.trim());
                if decoded_len > MAX_INLINE_IMAGE_BYTES {
                    return Err(ProtocolError::new(
                        ProtocolErrorKind::InvalidRequest,
                        format!("内联图片超过上限 {} 字节", MAX_INLINE_IMAGE_BYTES),
                    ));
                }
                Ok(())
            }
        }
    }

    fn is_text(&self) -> bool {
        matches!(self, Self::Text { .. })
    }

    fn is_empty_text(&self) -> bool {
        match self {
            Self::Text { text } => text.is_empty(),
            _ => false,
        }
    }
}

fn approx_base64_decoded_len(data: &str) -> usize {
    let clean: String = data.chars().filter(|c| !c.is_whitespace()).collect();
    let padding = clean.chars().rev().take_while(|c| *c == '=').count();
    clean.len().saturating_mul(3) / 4 - padding
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatRole {
    System,
    User,
    Assistant,
    Tool,
}

impl ChatRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
        }
    }
}

/// Assistant tool call (public semantic; wire mapping is protocol-specific).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub parts: Vec<ContentPart>,
    pub tool_call_id: Option<String>,
    pub tool_calls: Vec<ToolCall>,
}

impl ChatMessage {
    /// Convenience constructor for a single text part.
    pub fn text(role: ChatRole, content: impl Into<String>) -> Self {
        Self {
            role,
            parts: vec![ContentPart::Text {
                text: content.into(),
            }],
            tool_call_id: None,
            tool_calls: Vec::new(),
        }
    }

    /// Concatenate all text parts (V1 wire compatibility helper).
    pub fn text_content(&self) -> String {
        let mut out = String::new();
        for part in &self.parts {
            if let ContentPart::Text { text } = part {
                out.push_str(text);
            }
        }
        out
    }

    pub fn has_image_parts(&self) -> bool {
        self.parts.iter().any(|p| {
            matches!(
                p,
                ContentPart::ImageUrl { .. } | ContentPart::ImageBase64 { .. }
            )
        })
    }

    fn validate(&self) -> Result<(), ProtocolError> {
        // Assistant may carry only tool_calls (wire content null / empty).
        let assistant_tools_only = self.role == ChatRole::Assistant && !self.tool_calls.is_empty();
        if self.parts.is_empty() && !assistant_tools_only {
            return Err(ProtocolError::new(
                ProtocolErrorKind::InvalidRequest,
                "消息 parts 不能为空",
            ));
        }
        let only_empty_text =
            !self.parts.is_empty() && self.parts.iter().all(|p| p.is_text() && p.is_empty_text());
        if only_empty_text && !assistant_tools_only {
            return Err(ProtocolError::new(
                ProtocolErrorKind::InvalidRequest,
                "消息不能仅包含空 Text",
            ));
        }
        for part in &self.parts {
            part.validate()?;
        }
        for tc in &self.tool_calls {
            if tc.id.trim().is_empty() || tc.name.trim().is_empty() {
                return Err(ProtocolError::new(
                    ProtocolErrorKind::InvalidRequest,
                    "tool_calls 的 id/name 不能为空",
                ));
            }
        }
        if !self.tool_calls.is_empty() && self.role != ChatRole::Assistant {
            return Err(ProtocolError::new(
                ProtocolErrorKind::InvalidRequest,
                "仅 Assistant 消息可带 tool_calls",
            ));
        }
        match self.role {
            ChatRole::Tool => {
                if self
                    .tool_call_id
                    .as_ref()
                    .map(|s| s.trim().is_empty())
                    .unwrap_or(true)
                {
                    return Err(ProtocolError::new(
                        ProtocolErrorKind::InvalidRequest,
                        "Tool 消息必须提供 tool_call_id",
                    ));
                }
                if self.parts.iter().any(|p| !p.is_text()) {
                    return Err(ProtocolError::new(
                        ProtocolErrorKind::InvalidRequest,
                        "Tool 消息 parts 只能是 Text",
                    ));
                }
            }
            _ => {
                if self.tool_call_id.is_some() {
                    return Err(ProtocolError::new(
                        ProtocolErrorKind::InvalidRequest,
                        "非 Tool 消息不能带 tool_call_id",
                    ));
                }
            }
        }
        Ok(())
    }
}

/// Request-side reasoning control (not UI fold state).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasoningMode {
    Auto,
    Off,
    On,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasoningEffort {
    Minimal,
    Low,
    Medium,
    High,
    XHigh,
    Max,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReasoningPolicy {
    pub mode: ReasoningMode,
    pub effort: Option<ReasoningEffort>,
    pub budget_tokens: Option<u32>,
}

/// Tool definition for request wire (execution is out of scope).
#[derive(Debug, Clone, PartialEq)]
pub struct ToolDefinition {
    pub name: String,
    pub description: Option<String>,
    pub parameters: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolChoice {
    Auto,
    None,
    Required,
    Named { name: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub reasoning: Option<ReasoningPolicy>,
    pub tools: Option<Vec<ToolDefinition>>,
    pub tool_choice: Option<ToolChoice>,
}

impl ChatRequest {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.model.trim().is_empty() {
            return Err(ProtocolError::new(
                ProtocolErrorKind::InvalidRequest,
                "model 不能为空",
            ));
        }
        if !self.messages.iter().any(|m| m.role == ChatRole::User) {
            return Err(ProtocolError::new(
                ProtocolErrorKind::InvalidRequest,
                "messages 至少需要一条 user 消息",
            ));
        }
        for message in &self.messages {
            message.validate()?;
        }
        if let Some(temp) = self.temperature {
            if !(0.0..=2.0).contains(&temp) {
                return Err(ProtocolError::new(
                    ProtocolErrorKind::InvalidRequest,
                    "temperature 必须在 0.0..=2.0",
                ));
            }
        }
        if let Some(max_tokens) = self.max_tokens {
            if max_tokens == 0 {
                return Err(ProtocolError::new(
                    ProtocolErrorKind::InvalidRequest,
                    "max_tokens 必须大于 0",
                ));
            }
        }
        if let Some(policy) = &self.reasoning {
            validate_reasoning_policy(policy)?;
        }
        validate_tools(self)?;
        Ok(())
    }

    /// Capability gate for `stream_chat` (no HTTP if fails).
    pub fn check_capabilities(
        &self,
        capabilities: &ProtocolCapabilities,
    ) -> Result<(), ProtocolError> {
        if self.tools.is_some() && !capabilities.tools {
            return Err(ProtocolError::new(
                ProtocolErrorKind::Unsupported,
                "当前协议不支持 tools",
            ));
        }
        if self.messages.iter().any(ChatMessage::has_image_parts) && !capabilities.vision_input {
            return Err(ProtocolError::new(
                ProtocolErrorKind::Unsupported,
                "当前协议不支持视觉输入",
            ));
        }
        if let Some(policy) = &self.reasoning {
            let needs_control = matches!(policy.mode, ReasoningMode::On | ReasoningMode::Off);
            if needs_control && !capabilities.reasoning_control {
                return Err(ProtocolError::new(
                    ProtocolErrorKind::Unsupported,
                    "当前协议不支持推理控制",
                ));
            }
        }
        Ok(())
    }
}

fn validate_reasoning_policy(policy: &ReasoningPolicy) -> Result<(), ProtocolError> {
    // Auto / None-equivalent: effort/budget ignored at encode time; no extra error.
    if matches!(policy.mode, ReasoningMode::Auto) {
        return Ok(());
    }
    if let Some(budget) = policy.budget_tokens {
        if budget == 0 && policy.mode != ReasoningMode::Off {
            // Off may send budget 0 on Gemini; On with 0 is invalid at public layer
            // unless a protocol mapping uses it for Off only.
            return Err(ProtocolError::new(
                ProtocolErrorKind::InvalidRequest,
                "budget_tokens 在 On 模式下必须大于 0",
            ));
        }
    }
    Ok(())
}

fn validate_tools(request: &ChatRequest) -> Result<(), ProtocolError> {
    match (&request.tools, &request.tool_choice) {
        (None, Some(_)) => {
            return Err(ProtocolError::new(
                ProtocolErrorKind::InvalidRequest,
                "tools 为 None 时不能设置 tool_choice",
            ));
        }
        (None, None) => return Ok(()),
        (Some(tools), choice) => {
            if tools.is_empty() {
                return Err(ProtocolError::new(
                    ProtocolErrorKind::InvalidRequest,
                    "tools 不能为空列表",
                ));
            }
            let mut names = std::collections::HashSet::new();
            for tool in tools {
                let name = tool.name.trim();
                if name.is_empty() {
                    return Err(ProtocolError::new(
                        ProtocolErrorKind::InvalidRequest,
                        "tool name 不能为空",
                    ));
                }
                if !tool.parameters.is_object() {
                    return Err(ProtocolError::new(
                        ProtocolErrorKind::InvalidRequest,
                        "tool parameters 必须是 JSON object",
                    ));
                }
                if !names.insert(name.to_string()) {
                    return Err(ProtocolError::new(
                        ProtocolErrorKind::InvalidRequest,
                        format!("tool name 重复: {name}"),
                    ));
                }
            }
            if let Some(ToolChoice::Named { name }) = choice {
                let name = name.trim();
                if name.is_empty() || !names.contains(name) {
                    return Err(ProtocolError::new(
                        ProtocolErrorKind::InvalidRequest,
                        "tool_choice Named 必须引用已声明的 tool",
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Normalized completion reason for stream Completed events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinishReason {
    Stop,
    Length,
    ContentFilter,
    ToolCalls,
    Other(String),
}

impl FinishReason {
    /// Best-effort mapping for mixed/legacy call sites.
    ///
    /// Prefer protocol-specific helpers in the `finish` module (mapping-v2 tables):
    /// `finish_reason_openai_chat_completions`, `finish_reason_openai_responses`,
    /// `finish_reason_anthropic_messages`, `finish_reason_gemini_generate_content`.
    pub fn from_provider_raw(raw: Option<&str>) -> Self {
        let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
            return Self::Other("unknown".into());
        };
        // Case-insensitive common aliases across providers (not a substitute for mapping-v2).
        match raw.to_ascii_lowercase().as_str() {
            "stop" | "end_turn" | "completed" | "stop_sequence" => Self::Stop,
            "length" | "max_tokens" | "max_output_tokens" => Self::Length,
            "content_filter" | "content_filtered" | "safety" | "refusal" => Self::ContentFilter,
            "tool_calls" | "tool_use" | "function_call" => Self::ToolCalls,
            other => Self::Other(other.to_string()),
        }
    }

    /// Stable string for storage / IPC bridges that still expect a string.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Stop => "stop",
            Self::Length => "length",
            Self::ContentFilter => "content_filter",
            Self::ToolCalls => "tool_calls",
            Self::Other(s) => s.as_str(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TokenUsage {
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
    pub total_tokens: Option<u32>,
    pub reasoning_tokens: Option<u32>,
    pub cached_tokens: Option<u32>,
}

/// Adapter capability flags (must match real encode/decode support).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProtocolCapabilities {
    pub streaming: bool,
    pub list_models: bool,
    pub reasoning_output: bool,
    pub reasoning_control: bool,
    pub tools: bool,
    pub vision_input: bool,
    pub stream_usage: bool,
    pub custom_headers: bool,
    pub api_key_query: bool,
}

impl ProtocolCapabilities {
    /// Conservative V1 baseline: streaming + list_models + read-side reasoning where applicable.
    pub const fn v1_text_baseline() -> Self {
        Self {
            streaming: true,
            list_models: true,
            reasoning_output: true,
            reasoning_control: false,
            tools: false,
            vision_input: false,
            stream_usage: false,
            custom_headers: false,
            api_key_query: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEvent {
    ReasoningDelta {
        text: String,
    },
    ContentDelta {
        text: String,
    },
    ToolCallDelta {
        id: Option<String>,
        name: Option<String>,
        arguments_delta: String,
        index: Option<u32>,
    },
    ToolCallFinished {
        id: String,
        name: String,
        arguments: String,
        index: Option<u32>,
    },
    Usage(TokenUsage),
    Completed {
        finish_reason: FinishReason,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_base_url_and_keeps_v1() {
        let url = normalize_base_url("https://api.example.com/v1/").unwrap();
        assert_eq!(url.as_str(), "https://api.example.com/v1");
        let models = join_api_path(&url, "models").unwrap();
        assert_eq!(models.as_str(), "https://api.example.com/v1/models");
        let chat = join_api_path(&url, "chat/completions").unwrap();
        assert_eq!(chat.as_str(), "https://api.example.com/v1/chat/completions");
    }

    #[test]
    fn rejects_query_and_fragment() {
        assert!(normalize_base_url("https://api.example.com/v1?x=1").is_err());
        assert!(normalize_base_url("https://api.example.com/v1#frag").is_err());
    }

    #[test]
    fn validates_chat_request_text_ok() {
        let ok = ChatRequest {
            model: "gpt".into(),
            messages: vec![ChatMessage::text(ChatRole::User, "hi")],
            temperature: Some(1.0),
            max_tokens: Some(16),
            reasoning: None,
            tools: None,
            tool_choice: None,
        };
        assert!(ok.validate().is_ok());
    }

    #[test]
    fn validates_chat_request_failures() {
        let bad = ChatRequest {
            model: "  ".into(),
            messages: vec![ChatMessage::text(ChatRole::System, "sys")],
            temperature: Some(3.0),
            max_tokens: Some(0),
            reasoning: None,
            tools: None,
            tool_choice: None,
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn rejects_empty_parts_and_empty_text() {
        let empty_parts = ChatMessage {
            role: ChatRole::User,
            parts: vec![],
            tool_call_id: None,
            tool_calls: vec![],
        };
        assert!(empty_parts.validate().is_err());

        let empty_text = ChatMessage::text(ChatRole::User, "");
        assert!(empty_text.validate().is_err());
    }

    #[test]
    fn tool_role_rules() {
        let missing_id = ChatMessage {
            role: ChatRole::Tool,
            parts: vec![ContentPart::Text {
                text: "result".into(),
            }],
            tool_call_id: None,
            tool_calls: vec![],
        };
        assert!(missing_id.validate().is_err());

        let ok = ChatMessage {
            role: ChatRole::Tool,
            parts: vec![ContentPart::Text {
                text: "result".into(),
            }],
            tool_call_id: Some("call_1".into()),
            tool_calls: vec![],
        };
        assert!(ok.validate().is_ok());
    }

    #[test]
    fn reasoning_auto_ok_and_budget_zero_on_invalid() {
        let auto = ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessage::text(ChatRole::User, "hi")],
            temperature: None,
            max_tokens: None,
            reasoning: Some(ReasoningPolicy {
                mode: ReasoningMode::Auto,
                effort: Some(ReasoningEffort::High),
                budget_tokens: Some(0),
            }),
            tools: None,
            tool_choice: None,
        };
        assert!(auto.validate().is_ok());

        let on_zero = ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessage::text(ChatRole::User, "hi")],
            temperature: None,
            max_tokens: None,
            reasoning: Some(ReasoningPolicy {
                mode: ReasoningMode::On,
                effort: None,
                budget_tokens: Some(0),
            }),
            tools: None,
            tool_choice: None,
        };
        assert!(on_zero.validate().is_err());
    }

    #[test]
    fn capability_gate_rejects_tools_and_vision_and_reasoning() {
        let caps = ProtocolCapabilities::v1_text_baseline();
        let with_tools = ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessage::text(ChatRole::User, "hi")],
            temperature: None,
            max_tokens: None,
            reasoning: None,
            tools: Some(vec![ToolDefinition {
                name: "t".into(),
                description: None,
                parameters: serde_json::json!({}),
            }]),
            tool_choice: None,
        };
        assert_eq!(
            with_tools.check_capabilities(&caps).unwrap_err().kind,
            ProtocolErrorKind::Unsupported
        );

        let with_image = ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessage {
                role: ChatRole::User,
                parts: vec![
                    ContentPart::Text { text: "see".into() },
                    ContentPart::ImageUrl {
                        url: "https://example.com/a.png".into(),
                        detail: None,
                    },
                ],
                tool_call_id: None,
                tool_calls: vec![],
            }],
            temperature: None,
            max_tokens: None,
            reasoning: None,
            tools: None,
            tool_choice: None,
        };
        assert!(with_image.validate().is_ok());
        assert_eq!(
            with_image.check_capabilities(&caps).unwrap_err().kind,
            ProtocolErrorKind::Unsupported
        );

        let with_reason = ChatRequest {
            model: "m".into(),
            messages: vec![ChatMessage::text(ChatRole::User, "hi")],
            temperature: None,
            max_tokens: None,
            reasoning: Some(ReasoningPolicy {
                mode: ReasoningMode::On,
                effort: Some(ReasoningEffort::Low),
                budget_tokens: None,
            }),
            tools: None,
            tool_choice: None,
        };
        assert_eq!(
            with_reason.check_capabilities(&caps).unwrap_err().kind,
            ProtocolErrorKind::Unsupported
        );
    }

    #[test]
    fn oversized_inline_image_invalid_without_base64_body() {
        // ~5 MiB decoded payload as base64 padding approx: 7M 'A' chars → over 4 MiB.
        let huge = "A".repeat(7 * 1024 * 1024);
        let msg = ChatMessage {
            role: ChatRole::User,
            parts: vec![
                ContentPart::Text { text: "x".into() },
                ContentPart::ImageBase64 {
                    media_type: "image/png".into(),
                    data: huge.clone(),
                },
            ],
            tool_call_id: None,
            tool_calls: vec![],
        };
        let err = msg.validate().unwrap_err();
        assert_eq!(err.kind, ProtocolErrorKind::InvalidRequest);
        assert!(!err.message.contains(&huge[..64]));
        assert!(err.message.contains("内联图片超过上限"));
    }

    #[test]
    fn finish_reason_from_raw() {
        assert_eq!(
            FinishReason::from_provider_raw(Some("stop")),
            FinishReason::Stop
        );
        assert_eq!(
            FinishReason::from_provider_raw(Some("end_turn")),
            FinishReason::Stop
        );
        assert_eq!(
            FinishReason::from_provider_raw(Some("STOP")),
            FinishReason::Stop
        );
        assert_eq!(
            FinishReason::from_provider_raw(Some("tool_calls")),
            FinishReason::ToolCalls
        );
        assert_eq!(
            FinishReason::from_provider_raw(None),
            FinishReason::Other("unknown".into())
        );
    }
}
