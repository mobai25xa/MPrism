use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::storage::{
    MessageErrorRecord, MessageRecord, ModelRecord, ProviderPublic, SessionMeta, ThemePreference,
    TokenUsageRecord,
};

use super::AppError;

pub const IPC_SCHEMA_VERSION: u32 = 1;

/// Mirrors `mprism_protocol::ProtocolCapabilities` for frontend gating.
#[derive(Debug, Clone, Serialize)]
pub struct ProtocolCapabilitiesPayload {
    pub protocol: String,
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

/// Optional per-request reasoning control (SDK ReasoningPolicy).
#[derive(Debug, Clone, Deserialize)]
pub struct ReasoningPolicyInput {
    /// `auto` | `off` | `on`
    pub mode: String,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub budget_tokens: Option<u32>,
}

/// Tool definition from IPC / settings (maps to SDK `ToolDefinition`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinitionInput {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub parameters: serde_json::Value,
}

/// tool_choice from IPC / settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolChoiceInput {
    /// `auto` | `none` | `required` | `named`
    pub mode: String,
    #[serde(default)]
    pub name: Option<String>,
}

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
    /// Provider-level tools (3.6); omit/empty ≡ no tools wire.
    #[serde(default)]
    pub tools: Vec<crate::storage::StoredToolDefinition>,
    #[serde(default)]
    pub tool_choice: Option<crate::storage::StoredToolChoice>,
    /// Provider-level AuthOptions (3.7); omit/empty ≡ V1.
    #[serde(default)]
    pub extra_headers: Vec<crate::storage::StoredExtraHeader>,
    #[serde(default)]
    pub api_key_query_param: Option<String>,
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
    /// When omitted or mode auto → request `reasoning: None` (V1-compatible).
    #[serde(default)]
    pub reasoning: Option<ReasoningPolicyInput>,
    /// Image attachments (3.5).
    #[serde(default)]
    pub attachments: Option<Vec<ChatAttachmentInput>>,
    /// Optional per-request tools override; omit → use provider settings.
    #[serde(default)]
    pub tools: Option<Vec<ToolDefinitionInput>>,
    /// Optional per-request tool_choice override; omit → provider settings.
    #[serde(default)]
    pub tool_choice: Option<ToolChoiceInput>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChatAttachmentInput {
    /// Stored attachment id (from `import_attachment`).
    pub id: String,
    #[serde(default)]
    pub media_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImportAttachmentInput {
    pub schema_version: u32,
    /// Raw image bytes (frontend reads via File API; never logged).
    pub bytes: Vec<u8>,
    pub media_type: String,
    #[serde(default)]
    pub original_name: Option<String>,
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
    Usage {
        usage: TokenUsageRecord,
    },
    Completed {
        finish_reason: Option<String>,
    },
    Stopped,
    Error {
        error: MessageErrorRecord,
    },
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
