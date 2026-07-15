use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use mprism_protocol::{ChatMessage, ChatRole, ContentPart};
use uuid::Uuid;

use crate::storage::{
    AssistantStatus, FileStore, MessageAttachmentRef, MessageRecord, MessageRole, SessionMeta,
    SessionUpdate,
};

use super::{check_ipc_schema, AppError, LoadedSession, UpdateSessionInput, IPC_SCHEMA_VERSION};

pub struct SessionService {
    store: Arc<FileStore>,
}

impl SessionService {
    pub fn new(store: Arc<FileStore>) -> Self {
        Self { store }
    }

    pub fn create(&self, title: Option<String>) -> Result<SessionMeta, AppError> {
        self.store.create_session(title).map_err(AppError::from)
    }

    pub fn list(&self) -> Result<Vec<SessionMeta>, AppError> {
        self.store.list_sessions().map_err(AppError::from)
    }

    pub fn load(&self, session_id: Uuid) -> Result<LoadedSession, AppError> {
        let meta = self.store.load_session_meta(session_id)?;
        let loaded = self.store.load_messages(session_id)?;
        Ok(LoadedSession {
            schema_version: IPC_SCHEMA_VERSION,
            meta,
            messages: loaded.messages,
            partially_corrupt: loaded.partially_corrupt,
        })
    }

    pub fn update(
        &self,
        session_id: Uuid,
        input: UpdateSessionInput,
    ) -> Result<SessionMeta, AppError> {
        check_ipc_schema(input.schema_version)?;
        self.store
            .update_session(
                session_id,
                SessionUpdate {
                    title: input.title,
                    system_prompt: input.system_prompt,
                    last_provider_id: if input.set_last_provider_id {
                        Some(input.last_provider_id)
                    } else {
                        None
                    },
                    last_model_id: if input.set_last_model_id {
                        Some(input.last_model_id)
                    } else {
                        None
                    },
                },
            )
            .map_err(AppError::from)
    }

    pub fn delete(&self, session_id: Uuid) -> Result<(), AppError> {
        self.store.delete_session(session_id)?;
        Ok(())
    }

    pub fn build_context(&self, session_id: Uuid) -> Result<Vec<ChatMessage>, AppError> {
        let meta = self.store.load_session_meta(session_id)?;
        if meta.deleted_at.is_some() {
            return Err(AppError::new("not_found", "会话已删除", false));
        }
        let loaded = self.store.load_messages(session_id)?;
        let mut context = Vec::new();
        if !meta.system_prompt.trim().is_empty() {
            context.push(ChatMessage::text(
                ChatRole::System,
                meta.system_prompt.trim(),
            ));
        }
        for message in loaded.messages {
            match message.role {
                MessageRole::User => {
                    context.push(self.user_message_to_chat(&message)?);
                }
                MessageRole::Assistant => {
                    let include = matches!(
                        message.status,
                        Some(AssistantStatus::Completed | AssistantStatus::Stopped)
                    ) && !message.content.trim().is_empty();
                    if include {
                        context.push(ChatMessage::text(ChatRole::Assistant, message.content));
                    }
                }
            }
        }
        Ok(context)
    }

    fn user_message_to_chat(&self, message: &MessageRecord) -> Result<ChatMessage, AppError> {
        if message.attachments.is_empty() {
            return Ok(ChatMessage::text(ChatRole::User, message.content.clone()));
        }
        let mut parts = Vec::new();
        if !message.content.trim().is_empty() {
            parts.push(ContentPart::Text {
                text: message.content.clone(),
            });
        }
        for reference in &message.attachments {
            parts.push(self.attachment_to_image_part(reference)?);
        }
        if parts.is_empty() {
            return Err(AppError::validation("含附件的用户消息无效"));
        }
        Ok(ChatMessage {
            role: ChatRole::User,
            parts,
            tool_call_id: None,
            tool_calls: Vec::new(),
        })
    }

    fn attachment_to_image_part(
        &self,
        reference: &MessageAttachmentRef,
    ) -> Result<ContentPart, AppError> {
        let (meta, bytes) = self.store.load_attachment_bytes(reference.attachment_id)?;
        let media_type = reference
            .media_type
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(meta.media_type);
        // Always ImageBase64 for Gemini compatibility and offline local blobs.
        Ok(ContentPart::ImageBase64 {
            media_type,
            data: BASE64.encode(bytes),
        })
    }
}
