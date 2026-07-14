use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::storage::{
    MessageErrorRecord, MessageRecord, ModelRecord, ProviderPublic, SessionMeta, ThemePreference,
    TokenUsageRecord,
};

use super::AppError;

pub const IPC_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize)]
pub struct BootstrapPayload {
    pub schema_version: u32,
    pub theme: ThemePreference,
    pub default_provider_id: Option<Uuid>,
    pub default_model_id: Option<String>,
    pub providers: Vec<ProviderPublic>,
    pub sessions: Vec<SessionMeta>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LoadedSession {
    pub schema_version: u32,
    pub meta: SessionMeta,
    pub messages: Vec<MessageRecord>,
    pub partially_corrupt: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ApiKeyUpdateInput {
    Keep,
    Replace { value: String },
    Clear,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProviderInput {
    pub schema_version: u32,
    pub id: Option<Uuid>,
    pub name: String,
    pub protocol: String,
    pub base_url: String,
    pub api_key: ApiKeyUpdateInput,
    pub models: Vec<ModelRecord>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProviderDraft {
    pub schema_version: u32,
    pub provider_id: Option<Uuid>,
    pub protocol: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<ApiKeyUpdateInput>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelInfoPayload {
    pub id: String,
    pub display_name: String,
    pub owned_by: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateSessionInput {
    pub schema_version: u32,
    pub title: Option<String>,
    pub system_prompt: Option<String>,
    /// When true, apply last_provider_id (including null to clear).
    #[serde(default)]
    pub set_last_provider_id: bool,
    pub last_provider_id: Option<Uuid>,
    /// When true, apply last_model_id (including null to clear).
    #[serde(default)]
    pub set_last_model_id: bool,
    pub last_model_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChatInput {
    pub schema_version: u32,
    pub session_id: Uuid,
    pub provider_id: Uuid,
    pub model_id: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StreamEnvelope {
    pub schema_version: u32,
    pub request_id: Uuid,
    pub session_id: Uuid,
    pub assistant_message_id: Uuid,
    pub sequence: u64,
    pub event: StreamEventPayload,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEventPayload {
    Started,
    ReasoningDelta { text: String },
    ContentDelta { text: String },
    Usage { usage: TokenUsageRecord },
    Completed { finish_reason: Option<String> },
    Stopped,
    Error { error: MessageErrorRecord },
}

impl StreamEnvelope {
    pub fn new(
        request_id: Uuid,
        session_id: Uuid,
        assistant_message_id: Uuid,
        sequence: u64,
        event: StreamEventPayload,
    ) -> Self {
        Self {
            schema_version: IPC_SCHEMA_VERSION,
            request_id,
            session_id,
            assistant_message_id,
            sequence,
            event,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CancelChatPayload {
    pub schema_version: u32,
    pub was_running: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnitPayload {
    pub schema_version: u32,
}

pub fn check_ipc_schema(version: u32) -> Result<(), AppError> {
    if version != IPC_SCHEMA_VERSION {
        return Err(AppError::validation(format!(
            "不支持的 IPC schema_version: {version}"
        )));
    }
    Ok(())
}

