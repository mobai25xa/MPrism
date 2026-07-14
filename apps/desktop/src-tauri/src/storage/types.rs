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

#[derive(Clone, Serialize, Deserialize)]
pub struct ModelRecord {
    pub id: String,
    pub display_name: String,
    pub source: ModelSource,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
}

impl fmt::Debug for ModelRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModelRecord")
            .field("id", &self.id)
            .field("display_name", &self.display_name)
            .field("source", &self.source)
            .field("temperature", &self.temperature)
            .field("max_tokens", &self.max_tokens)
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
        let content = content.into();
        let trimmed = content.trim();
        if trimmed.is_empty() {
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
