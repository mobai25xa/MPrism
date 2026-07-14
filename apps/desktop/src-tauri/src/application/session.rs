use std::sync::Arc;

use mprism_protocol::{ChatMessage, ChatRole};
use uuid::Uuid;

use crate::storage::{AssistantStatus, FileStore, MessageRole, SessionMeta, SessionUpdate};

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
            context.push(ChatMessage {
                role: ChatRole::System,
                content: meta.system_prompt.trim().to_string(),
            });
        }
        for message in loaded.messages {
            match message.role {
                MessageRole::User => context.push(ChatMessage {
                    role: ChatRole::User,
                    content: message.content,
                }),
                MessageRole::Assistant => {
                    let include = matches!(
                        message.status,
                        Some(AssistantStatus::Completed | AssistantStatus::Stopped)
                    ) && !message.content.trim().is_empty();
                    if include {
                        context.push(ChatMessage {
                            role: ChatRole::Assistant,
                            content: message.content,
                        });
                    }
                }
            }
        }
        Ok(context)
    }
}

