//! Persistent document types for `.mprism` storage.

use std::fmt;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use super::error::{StorageError, StorageResult};
use super::paths::SCHEMA_VERSION;

pub const DEFAULT_SESSION_TITLE: &str = "新会话";
pub const MAX_PROVIDER_NAME_CHARS: usize = 80;
pub const MAX_SESSION_TITLE_CHARS: usize = 120;
pub const MAX_SYSTEM_PROMPT_CHARS: usize = 32_000;
pub const MAX_USER_CONTENT_CHARS: usize = 200_000;
pub const AUTO_TITLE_CHARS: usize = 30;

fn now_utc() -> OffsetDateTime {
    OffsetDateTime::now_utc()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemePreference {
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StoredProtocol {
    #[serde(rename = "openai_chat_completions")]
    OpenAiChatCompletions,
    #[serde(rename = "openai_responses")]
    OpenAiResponses,
    #[serde(rename = "anthropic_messages")]
    AnthropicMessages,
    #[serde(rename = "gemini_generate_content")]
    GeminiGenerateContent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelSource {
    Discovery,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TitleSource {
    Default,
    Auto,
    User,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantStatus {
    Completed,
    Stopped,
    Error,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct DeviceDocument {
    pub schema_version: u32,
    pub device_id: Uuid,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

impl fmt::Debug for DeviceDocument {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceDocument")
            .field("schema_version", &self.schema_version)
            .field("device_id", &self.device_id)
            .field("created_at", &self.created_at)
            .finish()
    }
}

impl DeviceDocument {
    pub fn new() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            device_id: Uuid::now_v7(),
            created_at: now_utc(),
        }
    }
}

impl Default for DeviceDocument {
    fn default() -> Self {
        Self::new()
    }
}

/// Request-side reasoning settings stored on a model (omit / mode auto ≡ V1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct StoredReasoningSettings {
    /// `auto` | `off` | `on`
    #[serde(default = "default_reasoning_mode")]
    pub mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_tokens: Option<u32>,
}

fn default_reasoning_mode() -> String {
    "auto".into()
}

impl StoredReasoningSettings {
    pub fn is_effective_none(&self) -> bool {
        matches!(self.mode.trim().to_ascii_lowercase().as_str(), "auto" | "")
            && self.effort.is_none()
            && self.budget_tokens.is_none()
    }

    pub fn validate(&self) -> StorageResult<()> {
        match self.mode.trim().to_ascii_lowercase().as_str() {
            "auto" | "off" | "on" => {}
            other => {
                return Err(StorageError::validation(format!(
                    "reasoning.mode 须为 auto/off/on，收到: {other}"
                )));
            }
        }
        if let Some(effort) = self
            .effort
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            match effort.to_ascii_lowercase().as_str() {
                "minimal" | "low" | "medium" | "high" | "xhigh" | "x_high" | "max" => {}
                other => {
                    return Err(StorageError::validation(format!(
                        "reasoning.effort 无效: {other}"
                    )));
                }
            }
        }
        if let Some(budget) = self.budget_tokens {
            if budget == 0 {
                return Err(StorageError::validation(
                    "reasoning.budget_tokens 必须为正整数或为空",
                ));
            }
        }
        Ok(())
    }
}

/// Provider-level tool definition (wire passthrough only; app does not execute).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredToolDefinition {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// JSON Schema object (must be object at validate time).
    pub parameters: serde_json::Value,
}

impl StoredToolDefinition {
    pub fn validate(&self) -> StorageResult<()> {
        if self.name.trim().is_empty() {
            return Err(StorageError::validation("tool name 不能为空"));
        }
        if !self.parameters.is_object() {
            return Err(StorageError::validation(
                "tool parameters 必须是 JSON object",
            ));
        }
        Ok(())
    }
}

/// Provider-level tool_choice. Absent / auto with empty tools ≡ V1 (no tools wire).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct StoredToolChoice {
    /// `auto` | `none` | `required` | `named`
    #[serde(default = "default_tool_choice_mode")]
    pub mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

fn default_tool_choice_mode() -> String {
    "auto".into()
}

impl StoredToolChoice {
    pub fn validate(&self, tool_names: &std::collections::HashSet<String>) -> StorageResult<()> {
        match self.mode.trim().to_ascii_lowercase().as_str() {
            "auto" | "none" | "required" => Ok(()),
            "named" => {
                let name = self
                    .name
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| StorageError::validation("tool_choice named 必须提供 name"))?;
                if !tool_names.contains(name) {
                    return Err(StorageError::validation(format!(
                        "tool_choice named 引用了未声明的 tool: {name}"
                    )));
                }
                Ok(())
            }
            other => Err(StorageError::validation(format!(
                "tool_choice.mode 须为 auto/none/required/named，收到: {other}"
            ))),
        }
    }
}

/// Validate tools list + choice together (SDK-aligned).
pub fn validate_stored_tools(
    tools: &[StoredToolDefinition],
    tool_choice: Option<&StoredToolChoice>,
) -> StorageResult<()> {
    if tools.is_empty() {
        if tool_choice.is_some() {
            return Err(StorageError::validation(
                "未配置 tools 时不能设置 tool_choice",
            ));
        }
        return Ok(());
    }
    let mut names = std::collections::HashSet::new();
    for tool in tools {
        tool.validate()?;
        let name = tool.name.trim().to_string();
        if !names.insert(name.clone()) {
            return Err(StorageError::validation(format!("tool name 重复: {name}")));
        }
    }
    if let Some(choice) = tool_choice {
        choice.validate(&names)?;
    }
    Ok(())
}

/// Extra HTTP header stored on provider (value redacted in Debug).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredExtraHeader {
    pub name: String,
    pub value: String,
}

impl fmt::Debug for StoredExtraHeader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoredExtraHeader")
            .field("name", &self.name)
            .field("value", &"***")
            .finish()
    }
}

impl StoredExtraHeader {
    pub fn validate(&self) -> StorageResult<()> {
        // Check CR/LF on raw input before trim (trailing newlines must not pass).
        if self.name.chars().any(|c| c == '\r' || c == '\n') {
            return Err(StorageError::validation("extra_headers 名称不能包含 CR/LF"));
        }
        if self.value.chars().any(|c| c == '\r' || c == '\n') {
            return Err(StorageError::validation("extra_headers 值不能包含 CR/LF"));
        }
        if self.name.trim().is_empty() {
            return Err(StorageError::validation("extra_headers 名称不能为空"));
        }
        Ok(())
    }
}

/// Validate provider auth extensions (SDK AuthOptions-aligned).
pub fn validate_stored_auth(
    extra_headers: &[StoredExtraHeader],
    api_key_query_param: Option<&str>,
) -> StorageResult<()> {
    for header in extra_headers {
        header.validate()?;
    }
    if let Some(param) = api_key_query_param {
        if param
            .chars()
            .any(|c| c == '\r' || c == '\n' || c == '=' || c == '&')
        {
            return Err(StorageError::validation("api_key_query_param 包含非法字符"));
        }
        if param.trim().is_empty() {
            return Err(StorageError::validation(
                "api_key_query_param 不能为空字符串（可省略）",
            ));
        }
    }
    Ok(())
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ModelRecord {
    pub id: String,
    pub display_name: String,
    pub source: ModelSource,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// Model-level reasoning policy; absent on old settings ≡ auto/None.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<StoredReasoningSettings>,
}

impl fmt::Debug for ModelRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModelRecord")
            .field("id", &self.id)
            .field("display_name", &self.display_name)
            .field("source", &self.source)
            .field("temperature", &self.temperature)
            .field("max_tokens", &self.max_tokens)
            .field("reasoning", &self.reasoning)
            .finish()
    }
}

impl ModelRecord {
    pub fn validate(&self) -> StorageResult<()> {
        if self.id.trim().is_empty() {
            return Err(StorageError::validation("model id 不能为空"));
        }
        if self.display_name.trim().is_empty() {
            return Err(StorageError::validation("model display_name 不能为空"));
        }
        if let Some(t) = self.temperature {
            if !(0.0..=2.0).contains(&t) {
                return Err(StorageError::validation(
                    "temperature 必须在 0.0..=2.0 或为空",
                ));
            }
        }
        if let Some(m) = self.max_tokens {
            if m == 0 {
                return Err(StorageError::validation("max_tokens 必须为正整数或为空"));
            }
        }
        if let Some(reasoning) = &self.reasoning {
            reasoning.validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ProviderRecord {
    pub id: Uuid,
    pub name: String,
    pub protocol: StoredProtocol,
    pub base_url: String,
    pub api_key: String,
    pub models: Vec<ModelRecord>,
    /// Provider-level tools (3.6); empty ≡ V1 no tools wire.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<StoredToolDefinition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<StoredToolChoice>,
    /// Provider-level AuthOptions (3.7); empty ≡ V1.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_headers: Vec<StoredExtraHeader>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_query_param: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    pub revision: u64,
}

impl fmt::Debug for ProviderRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderRecord")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("protocol", &self.protocol)
            .field("base_url", &self.base_url)
            .field(
                "api_key",
                &if self.api_key.is_empty() {
                    "<empty>"
                } else {
                    "***"
                },
            )
            .field("models", &self.models)
            .field("tools_len", &self.tools.len())
            .field("tool_choice", &self.tool_choice)
            .field("extra_headers", &self.extra_headers)
            .field("api_key_query_param", &self.api_key_query_param)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .field("revision", &self.revision)
            .finish()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SettingsDocument {
    pub schema_version: u32,
    pub theme: ThemePreference,
    #[serde(default)]
    pub default_provider_id: Option<Uuid>,
    #[serde(default)]
    pub default_model_id: Option<String>,
    pub providers: Vec<ProviderRecord>,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    pub revision: u64,
}

impl fmt::Debug for SettingsDocument {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SettingsDocument")
            .field("schema_version", &self.schema_version)
            .field("theme", &self.theme)
            .field("default_provider_id", &self.default_provider_id)
            .field("default_model_id", &self.default_model_id)
            .field("providers", &self.providers)
            .field("updated_at", &self.updated_at)
            .field("revision", &self.revision)
            .finish()
    }
}

impl SettingsDocument {
    pub fn new() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            theme: ThemePreference::System,
            default_provider_id: None,
            default_model_id: None,
            providers: Vec::new(),
            updated_at: now_utc(),
            revision: 1,
        }
    }

    pub fn touch(&mut self) {
        self.revision = self.revision.saturating_add(1);
        self.updated_at = now_utc();
    }

    /// Repair default provider/model pair after mutations.
    pub fn repair_defaults(&mut self) {
        let valid = self.default_provider_id.and_then(|pid| {
            self.providers.iter().find(|p| p.id == pid).and_then(|p| {
                self.default_model_id.as_ref().and_then(|mid| {
                    if p.models.iter().any(|m| m.id == *mid) {
                        Some((pid, mid.clone()))
                    } else {
                        None
                    }
                })
            })
        });

        if valid.is_some() {
            return;
        }

        if let Some(p) = self.providers.first() {
            if let Some(m) = p.models.first() {
                self.default_provider_id = Some(p.id);
                self.default_model_id = Some(m.id.clone());
                return;
            }
        }
        self.default_provider_id = None;
        self.default_model_id = None;
    }
}

impl Default for SettingsDocument {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub schema_version: u32,
    pub id: Uuid,
    pub title: String,
    pub title_source: TitleSource,
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default)]
    pub last_provider_id: Option<Uuid>,
    #[serde(default)]
    pub last_model_id: Option<String>,
    pub created_by_device_id: Uuid,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    pub revision: u64,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub deleted_at: Option<OffsetDateTime>,
}

impl SessionMeta {
    pub fn new(device_id: Uuid) -> Self {
        let now = now_utc();
        Self {
            schema_version: SCHEMA_VERSION,
            id: Uuid::now_v7(),
            title: DEFAULT_SESSION_TITLE.to_string(),
            title_source: TitleSource::Default,
            system_prompt: String::new(),
            last_provider_id: None,
            last_model_id: None,
            created_by_device_id: device_id,
            created_at: now,
            updated_at: now,
            revision: 1,
            deleted_at: None,
        }
    }

    pub fn touch(&mut self) {
        self.revision = self.revision.saturating_add(1);
        self.updated_at = now_utc();
    }

    pub fn validate(&self) -> StorageResult<()> {
        let title = self.title.trim();
        let chars = title.chars().count();
        if chars == 0 || chars > MAX_SESSION_TITLE_CHARS {
            return Err(StorageError::validation(format!(
                "会话标题长度须为 1–{MAX_SESSION_TITLE_CHARS} 个字符"
            )));
        }
        let sp = self.system_prompt.chars().count();
        if sp > MAX_SYSTEM_PROMPT_CHARS {
            return Err(StorageError::validation(format!(
                "system prompt 最长 {MAX_SYSTEM_PROMPT_CHARS} 个字符"
            )));
        }
        Ok(())
    }

    /// Auto-title from first non-empty user message when still default.
    pub fn maybe_auto_title(&mut self, user_content: &str) {
        if self.title_source != TitleSource::Default {
            return;
        }
        let trimmed = user_content.trim();
        if trimmed.is_empty() {
            return;
        }
        let title: String = trimmed.chars().take(AUTO_TITLE_CHARS).collect();
        if title.is_empty() {
            return;
        }
        self.title = title;
        self.title_source = TitleSource::Auto;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSnapshot {
    pub id: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSnapshot {
    pub id: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsageRecord {
    #[serde(default)]
    pub prompt_tokens: Option<u32>,
    #[serde(default)]
    pub completion_tokens: Option<u32>,
    #[serde(default)]
    pub total_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageErrorRecord {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    #[serde(default)]
    pub http_status: Option<u16>,
    #[serde(default)]
    pub provider_request_id: Option<String>,
    /// Optional Retry-After from provider (ms). UI may surface only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
}

/// Persisted tool call snapshot (display only; app does not execute).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>,
}

/// Message-side attachment reference (blob lives under attachments/).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageAttachmentRef {
    pub attachment_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct MessageRecord {
    pub schema_version: u32,
    pub id: Uuid,
    pub session_id: Uuid,
    pub sequence: u64,
    pub role: MessageRole,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<AssistantStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsageRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    /// Tool calls observed in stream (3.4+); empty on legacy messages.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<StoredToolCall>,
    /// Image attachment refs (3.5+); never stores base64 inline.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<MessageAttachmentRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<MessageErrorRecord>,
    pub created_by_device_id: Uuid,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(
        default,
        with = "time::serde::rfc3339::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub completed_at: Option<OffsetDateTime>,
}

impl fmt::Debug for MessageRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never dump full content/reasoning in Debug (logs may print Debug).
        f.debug_struct("MessageRecord")
            .field("schema_version", &self.schema_version)
            .field("id", &self.id)
            .field("session_id", &self.session_id)
            .field("sequence", &self.sequence)
            .field("role", &self.role)
            .field("content_len", &self.content.chars().count())
            .field(
                "reasoning_len",
                &self.reasoning.as_ref().map(|s| s.chars().count()),
            )
            .field("status", &self.status)
            .field("request_id", &self.request_id)
            .field("provider", &self.provider)
            .field("model", &self.model)
            .field("usage", &self.usage)
            .field("finish_reason", &self.finish_reason)
            .field("tool_calls_len", &self.tool_calls.len())
            .field("attachments_len", &self.attachments.len())
            .field("error", &self.error)
            .field("created_by_device_id", &self.created_by_device_id)
            .field("created_at", &self.created_at)
            .field("completed_at", &self.completed_at)
            .finish()
    }
}

impl MessageRecord {
    pub fn new_user(
        session_id: Uuid,
        sequence: u64,
        content: impl Into<String>,
        device_id: Uuid,
    ) -> StorageResult<Self> {
        Self::new_user_with_attachments(session_id, sequence, content, Vec::new(), device_id)
    }

    pub fn new_user_with_attachments(
        session_id: Uuid,
        sequence: u64,
        content: impl Into<String>,
        attachments: Vec<MessageAttachmentRef>,
        device_id: Uuid,
    ) -> StorageResult<Self> {
        let content = content.into();
        let trimmed = content.trim();
        if trimmed.is_empty() && attachments.is_empty() {
            return Err(StorageError::validation("用户消息不能为空"));
        }
        if trimmed.chars().count() > MAX_USER_CONTENT_CHARS {
            return Err(StorageError::validation(format!(
                "用户消息最长 {MAX_USER_CONTENT_CHARS} 个字符"
            )));
        }
        Ok(Self {
            schema_version: SCHEMA_VERSION,
            id: Uuid::now_v7(),
            session_id,
            sequence,
            role: MessageRole::User,
            content: trimmed.to_string(),
            reasoning: None,
            status: None,
            request_id: None,
            provider: None,
            model: None,
            usage: None,
            finish_reason: None,
            tool_calls: Vec::new(),
            attachments,
            error: None,
            created_by_device_id: device_id,
            created_at: now_utc(),
            completed_at: None,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_assistant(
        session_id: Uuid,
        sequence: u64,
        content: impl Into<String>,
        reasoning: Option<String>,
        status: AssistantStatus,
        request_id: Uuid,
        provider: ProviderSnapshot,
        model: ModelSnapshot,
        usage: Option<TokenUsageRecord>,
        finish_reason: Option<String>,
        tool_calls: Vec<StoredToolCall>,
        error: Option<MessageErrorRecord>,
        device_id: Uuid,
    ) -> Self {
        let completed_at = Some(now_utc());
        Self {
            schema_version: SCHEMA_VERSION,
            id: Uuid::now_v7(),
            session_id,
            sequence,
            role: MessageRole::Assistant,
            content: content.into(),
            reasoning,
            status: Some(status),
            request_id: Some(request_id),
            provider: Some(provider),
            model: Some(model),
            usage,
            finish_reason,
            tool_calls,
            attachments: Vec::new(),
            error,
            created_by_device_id: device_id,
            created_at: now_utc(),
            completed_at,
        }
    }

    pub fn with_id(mut self, id: Uuid) -> Self {
        self.id = id;
        self
    }
}

/// Normalize base URL for storage (no query/fragment, strip trailing slash).
pub fn normalize_stored_base_url(raw: &str) -> StorageResult<String> {
    let url = mprism_protocol::normalize_base_url(raw)
        .map_err(|e| StorageError::validation(e.to_string()))?;
    Ok(url.to_string().trim_end_matches('/').to_string())
}

pub fn char_len(s: &str) -> usize {
    s.chars().count()
}
