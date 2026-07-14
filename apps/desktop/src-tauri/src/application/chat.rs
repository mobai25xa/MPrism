use std::sync::Arc;

use futures_util::StreamExt;
use mprism_protocol::{
    ChatRequest, ProtocolAdapter, ProviderEndpoint, SecretString, StreamEvent, TokenUsage,
};
use parking_lot::RwLock;
use uuid::Uuid;

use crate::state::AdapterRegistry;
use crate::storage::{
    AssistantStatus, FileStore, MessageRecord, ModelSnapshot, ProviderSnapshot, SettingsDocument,
    TokenUsageRecord,
};

use super::{
    check_ipc_schema, protocol_kind, AppError, ChatInput, GenerationManager, SessionService,
    StreamEnvelope, StreamEventPayload,
};

pub trait StreamSink: Send + Sync {
    fn send(&self, envelope: StreamEnvelope) -> Result<(), AppError>;
}

pub struct ChatService {
    store: Arc<FileStore>,
    settings: Arc<RwLock<SettingsDocument>>,
    adapters: Arc<AdapterRegistry>,
    sessions: Arc<SessionService>,
    generations: Arc<GenerationManager>,
}

struct SelectedModel {
    endpoint: ProviderEndpoint,
    adapter: Arc<dyn ProtocolAdapter>,
    provider: ProviderSnapshot,
    model: ModelSnapshot,
    temperature: Option<f32>,
    max_tokens: Option<u32>,
}

impl ChatService {
    pub fn new(
        store: Arc<FileStore>,
        settings: Arc<RwLock<SettingsDocument>>,
        adapters: Arc<AdapterRegistry>,
        sessions: Arc<SessionService>,
        generations: Arc<GenerationManager>,
    ) -> Self {
        Self {
            store,
            settings,
            adapters,
            sessions,
            generations,
        }
    }

    pub async fn start_chat(
        &self,
        input: ChatInput,
        sink: &dyn StreamSink,
    ) -> Result<MessageRecord, AppError> {
        check_ipc_schema(input.schema_version)?;
        let selected = self.resolve_selected_model(input.provider_id, &input.model_id)?;
        let request_id = Uuid::now_v7();
        let assistant_message_id = Uuid::now_v7();
        let (_guard, cancellation) = self.generations.register(input.session_id, request_id)?;

        let user =
            MessageRecord::new_user(input.session_id, 0, input.content, self.store.device_id())?;
        self.store.append_message_and_touch(user, true)?;

        let context = self.sessions.build_context(input.session_id)?;
        let request = ChatRequest {
            model: input.model_id,
            messages: context,
            temperature: selected.temperature,
            max_tokens: selected.max_tokens,
        };
        request.validate()?;

        let mut sequence = 0;
        if send_event(
            sink,
            request_id,
            input.session_id,
            assistant_message_id,
            &mut sequence,
            StreamEventPayload::Started,
        )
        .is_err()
        {
            cancellation.cancel();
        }

        let stream_result = tokio::select! {
            _ = cancellation.cancelled() => None,
            result = selected.adapter.stream_chat(&selected.endpoint, request) => Some(result),
        };

        let mut content = String::new();
        let mut reasoning = String::new();
        let mut usage = None;
        let mut finish_reason = None;
        let mut final_status = AssistantStatus::Completed;
        let mut final_error: Option<AppError> = None;

        if let Some(stream_result) = stream_result {
            match stream_result {
                Ok(mut stream) => loop {
                    let next = tokio::select! {
                        _ = cancellation.cancelled() => {
                            final_status = AssistantStatus::Stopped;
                            break;
                        }
                        item = stream.next() => item,
                    };
                    match next {
                        Some(Ok(StreamEvent::ReasoningDelta { text })) => {
                            reasoning.push_str(&text);
                            if send_event(
                                sink,
                                request_id,
                                input.session_id,
                                assistant_message_id,
                                &mut sequence,
                                StreamEventPayload::ReasoningDelta { text },
                            )
                            .is_err()
                            {
                                cancellation.cancel();
                            }
                        }
                        Some(Ok(StreamEvent::ContentDelta { text })) => {
                            content.push_str(&text);
                            if send_event(
                                sink,
                                request_id,
                                input.session_id,
                                assistant_message_id,
                                &mut sequence,
                                StreamEventPayload::ContentDelta { text },
                            )
                            .is_err()
                            {
                                cancellation.cancel();
                            }
                        }
                        Some(Ok(StreamEvent::Usage(value))) => {
                            let record = usage_record(value);
                            usage = Some(record.clone());
                            if send_event(
                                sink,
                                request_id,
                                input.session_id,
                                assistant_message_id,
                                &mut sequence,
                                StreamEventPayload::Usage { usage: record },
                            )
                            .is_err()
                            {
                                cancellation.cancel();
                            }
                        }
                        Some(Ok(StreamEvent::Completed {
                            finish_reason: value,
                        })) => {
                            finish_reason = value.clone();
                            let _ = send_event(
                                sink,
                                request_id,
                                input.session_id,
                                assistant_message_id,
                                &mut sequence,
                                StreamEventPayload::Completed {
                                    finish_reason: value,
                                },
                            );
                            break;
                        }
                        Some(Err(error)) => {
                            final_status = AssistantStatus::Error;
                            final_error = Some(error.into());
                            break;
                        }
                        None => {
                            final_status = AssistantStatus::Error;
                            final_error =
                                Some(AppError::new("protocol", "模型流在完成事件前结束", false));
                            break;
                        }
                    }
                },
                Err(error) => {
                    final_status = AssistantStatus::Error;
                    final_error = Some(error.into());
                }
            }
        } else {
            final_status = AssistantStatus::Stopped;
        }

        if cancellation.is_cancelled() && final_status != AssistantStatus::Error {
            final_status = AssistantStatus::Stopped;
        }

        match final_status {
            AssistantStatus::Stopped => {
                let _ = send_event(
                    sink,
                    request_id,
                    input.session_id,
                    assistant_message_id,
                    &mut sequence,
                    StreamEventPayload::Stopped,
                );
            }
            AssistantStatus::Error => {
                let error = final_error
                    .as_ref()
                    .cloned()
                    .unwrap_or_else(|| AppError::new("internal", "生成失败", false));
                let _ = send_event(
                    sink,
                    request_id,
                    input.session_id,
                    assistant_message_id,
                    &mut sequence,
                    StreamEventPayload::Error {
                        error: error.to_message_record(),
                    },
                );
            }
            AssistantStatus::Completed => {}
        }

        let message = MessageRecord::new_assistant(
            input.session_id,
            0,
            content,
            if reasoning.is_empty() {
                None
            } else {
                Some(reasoning)
            },
            final_status,
            request_id,
            selected.provider,
            selected.model,
            usage,
            finish_reason,
            final_error.map(|error| error.to_message_record()),
            self.store.device_id(),
        )
        .with_id(assistant_message_id);

        self.store
            .append_message_and_touch(message, false)
            .map_err(AppError::from)
    }

    pub fn cancel(&self, request_id: Uuid) -> bool {
        self.generations.cancel(request_id)
    }

    fn resolve_selected_model(
        &self,
        provider_id: Uuid,
        model_id: &str,
    ) -> Result<SelectedModel, AppError> {
        let settings = self.settings.read();
        let provider = settings
            .providers
            .iter()
            .find(|provider| provider.id == provider_id)
            .ok_or_else(|| AppError::new("not_found", "服务商不存在", false))?;
        let model = provider
            .models
            .iter()
            .find(|model| model.id == model_id)
            .ok_or_else(|| AppError::new("not_found", "模型不存在", false))?;
        let kind = protocol_kind(provider.protocol);
        let endpoint = ProviderEndpoint::new(
            kind,
            &provider.base_url,
            SecretString::new(provider.api_key.clone()),
        )?;
        Ok(SelectedModel {
            adapter: self.adapters.get(kind)?,
            endpoint,
            provider: ProviderSnapshot {
                id: provider.id,
                name: provider.name.clone(),
            },
            model: ModelSnapshot {
                id: model.id.clone(),
                display_name: model.display_name.clone(),
            },
            temperature: model.temperature,
            max_tokens: model.max_tokens,
        })
    }
}

fn usage_record(usage: TokenUsage) -> TokenUsageRecord {
    TokenUsageRecord {
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        total_tokens: usage.total_tokens,
    }
}

fn send_event(
    sink: &dyn StreamSink,
    request_id: Uuid,
    session_id: Uuid,
    assistant_message_id: Uuid,
    sequence: &mut u64,
    event: StreamEventPayload,
) -> Result<(), AppError> {
    let current = *sequence;
    *sequence = sequence.saturating_add(1);
    sink.send(StreamEnvelope::new(
        request_id,
        session_id,
        assistant_message_id,
        current,
        event,
    ))
}
