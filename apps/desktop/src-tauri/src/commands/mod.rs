use tauri::ipc::Channel;
use tauri::State;
use uuid::Uuid;

use crate::application::check_ipc_schema;
use crate::application::{
    AppError, BootstrapPayload, CancelChatPayload, ChatInput, ImportAttachmentInput, LoadedSession,
    ModelInfoPayload, ProtocolCapabilitiesPayload, ProviderDraft, ProviderInput, StreamEnvelope,
    StreamSink, UnitPayload, UpdateSessionInput, IPC_SCHEMA_VERSION,
};
use crate::state::AppState;
use crate::storage::{
    AttachmentPublic, MessageRecord, ProviderPublic, SessionMeta, ThemePreference,
};

#[tauri::command]
pub fn bootstrap(state: State<'_, AppState>) -> Result<BootstrapPayload, AppError> {
    let settings = state.settings.read();
    Ok(BootstrapPayload {
        schema_version: IPC_SCHEMA_VERSION,
        theme: settings.theme,
        default_provider_id: settings.default_provider_id,
        default_model_id: settings.default_model_id.clone(),
        providers: state.providers.providers(),
        sessions: state.sessions.list()?,
    })
}

#[tauri::command]
pub fn set_theme(
    state: State<'_, AppState>,
    theme: ThemePreference,
) -> Result<ThemePreference, AppError> {
    state.providers.set_theme(theme)
}

#[tauri::command]
pub fn upsert_provider(
    state: State<'_, AppState>,
    input: ProviderInput,
) -> Result<ProviderPublic, AppError> {
    state.providers.upsert(input)
}

#[tauri::command]
pub fn delete_provider(
    state: State<'_, AppState>,
    provider_id: Uuid,
) -> Result<UnitPayload, AppError> {
    state.providers.delete(provider_id)?;
    Ok(UnitPayload {
        schema_version: IPC_SCHEMA_VERSION,
    })
}

#[tauri::command]
pub fn set_defaults(
    state: State<'_, AppState>,
    provider_id: Option<Uuid>,
    model_id: Option<String>,
) -> Result<UnitPayload, AppError> {
    state.providers.set_defaults(provider_id, model_id)?;
    Ok(UnitPayload {
        schema_version: IPC_SCHEMA_VERSION,
    })
}

#[tauri::command]
pub async fn discover_models(
    state: State<'_, AppState>,
    draft: ProviderDraft,
) -> Result<Vec<ModelInfoPayload>, AppError> {
    state.providers.discover_models(draft).await
}

#[tauri::command]
pub fn list_protocol_capabilities(
    state: State<'_, AppState>,
) -> Result<Vec<ProtocolCapabilitiesPayload>, AppError> {
    Ok(state.providers.list_protocol_capabilities())
}

#[tauri::command]
pub fn get_protocol_capabilities(
    state: State<'_, AppState>,
    protocol: String,
) -> Result<ProtocolCapabilitiesPayload, AppError> {
    state.providers.protocol_capabilities(&protocol)
}

#[tauri::command]
pub fn create_session(
    state: State<'_, AppState>,
    title: Option<String>,
) -> Result<SessionMeta, AppError> {
    state.sessions.create(title)
}

#[tauri::command]
pub fn list_sessions(state: State<'_, AppState>) -> Result<Vec<SessionMeta>, AppError> {
    state.sessions.list()
}

#[tauri::command]
pub fn load_session(
    state: State<'_, AppState>,
    session_id: Uuid,
) -> Result<LoadedSession, AppError> {
    state.sessions.load(session_id)
}

#[tauri::command]
pub fn update_session(
    state: State<'_, AppState>,
    session_id: Uuid,
    input: UpdateSessionInput,
) -> Result<SessionMeta, AppError> {
    state.sessions.update(session_id, input)
}

#[tauri::command]
pub fn delete_session(
    state: State<'_, AppState>,
    session_id: Uuid,
) -> Result<UnitPayload, AppError> {
    state.sessions.delete(session_id)?;
    Ok(UnitPayload {
        schema_version: IPC_SCHEMA_VERSION,
    })
}

struct ChannelSink {
    channel: Channel<StreamEnvelope>,
}

impl StreamSink for ChannelSink {
    fn send(&self, envelope: StreamEnvelope) -> Result<(), AppError> {
        self.channel
            .send(envelope)
            .map_err(|_| AppError::cancelled())
    }
}

#[tauri::command]
pub fn import_attachment(
    state: State<'_, AppState>,
    input: ImportAttachmentInput,
) -> Result<AttachmentPublic, AppError> {
    check_ipc_schema(input.schema_version)?;
    state
        .store
        .import_attachment(&input.bytes, &input.media_type, input.original_name)
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn start_chat(
    state: State<'_, AppState>,
    input: ChatInput,
    on_event: Channel<StreamEnvelope>,
) -> Result<MessageRecord, AppError> {
    state
        .chat
        .start_chat(input, &ChannelSink { channel: on_event })
        .await
}

#[tauri::command]
pub fn cancel_chat(
    state: State<'_, AppState>,
    request_id: Uuid,
) -> Result<CancelChatPayload, AppError> {
    Ok(CancelChatPayload {
        schema_version: IPC_SCHEMA_VERSION,
        was_running: state.chat.cancel(request_id),
    })
}
