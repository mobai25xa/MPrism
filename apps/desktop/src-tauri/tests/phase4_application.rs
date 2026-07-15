use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures_util::stream;
use mprism_desktop_lib::application::{
    ApiKeyUpdateInput, ChatInput, GenerationManager, ProviderInput, StreamEnvelope, StreamSink,
    IPC_SCHEMA_VERSION,
};
use mprism_desktop_lib::state::{AdapterRegistry, AppState};
use mprism_desktop_lib::storage::{
    AssistantStatus, FileStore, MessageErrorRecord, MessageRecord, ModelRecord, ModelSnapshot,
    ModelSource, ProviderSnapshot,
};
use mprism_protocol::{
    ChatRequest, ChatStream, FinishReason, ModelInfo, ProtocolAdapter, ProtocolCapabilities,
    ProtocolError, ProtocolErrorKind, ProtocolKind, ProviderEndpoint, StreamEvent, TokenUsage,
};
use tokio::sync::Notify;
use uuid::Uuid;

#[derive(Clone)]
enum FakeMode {
    Success,
    Fail,
    Blocking(Arc<Notify>),
}

struct FakeAdapter {
    kind: ProtocolKind,
    mode: FakeMode,
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}

#[async_trait]
impl ProtocolAdapter for FakeAdapter {
    fn kind(&self) -> ProtocolKind {
        self.kind
    }

    fn capabilities(&self) -> ProtocolCapabilities {
        ProtocolCapabilities::v1_text_baseline()
    }

    async fn list_models(
        &self,
        _endpoint: &ProviderEndpoint,
    ) -> Result<Vec<ModelInfo>, ProtocolError> {
        Ok(vec![ModelInfo {
            id: "fake-model".into(),
            display_name: "Fake Model".into(),
            owned_by: Some("test".into()),
        }])
    }

    async fn stream_chat(
        &self,
        _endpoint: &ProviderEndpoint,
        request: ChatRequest,
    ) -> Result<ChatStream, ProtocolError> {
        self.requests.lock().unwrap().push(request);
        match &self.mode {
            FakeMode::Success => Ok(Box::pin(stream::iter(vec![
                Ok(StreamEvent::ReasoningDelta {
                    text: "think".into(),
                }),
                Ok(StreamEvent::ContentDelta {
                    text: "hello".into(),
                }),
                Ok(StreamEvent::Usage(TokenUsage {
                    prompt_tokens: Some(3),
                    completion_tokens: Some(2),
                    total_tokens: Some(5),
                    reasoning_tokens: None,
                    cached_tokens: None,
                })),
                Ok(StreamEvent::Completed {
                    finish_reason: FinishReason::Stop,
                }),
            ]))),
            FakeMode::Fail => Err(ProtocolError::new(
                ProtocolErrorKind::ProviderUnavailable,
                "upstream unavailable",
            )),
            FakeMode::Blocking(notify) => {
                let notify = Arc::clone(notify);
                Ok(Box::pin(stream::once(async move {
                    notify.notified().await;
                    Ok(StreamEvent::ContentDelta {
                        text: "late".into(),
                    })
                })))
            }
        }
    }
}

#[derive(Default)]
struct CaptureSink {
    events: Mutex<Vec<StreamEnvelope>>,
    fail_after: Option<usize>,
}

impl StreamSink for CaptureSink {
    fn send(
        &self,
        envelope: StreamEnvelope,
    ) -> Result<(), mprism_desktop_lib::application::AppError> {
        let mut events = self.events.lock().unwrap();
        if self.fail_after.is_some_and(|limit| events.len() >= limit) {
            return Err(mprism_desktop_lib::application::AppError::cancelled());
        }
        events.push(envelope);
        Ok(())
    }
}

fn test_state(
    mode: FakeMode,
) -> (
    tempfile::TempDir,
    Arc<AppState>,
    Arc<Mutex<Vec<ChatRequest>>>,
) {
    test_state_for_protocol(mode, ProtocolKind::OpenAiChatCompletions)
}

fn test_state_for_protocol(
    mode: FakeMode,
    kind: ProtocolKind,
) -> (
    tempfile::TempDir,
    Arc<AppState>,
    Arc<Mutex<Vec<ChatRequest>>>,
) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(FileStore::open(dir.path()).unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut registry = AdapterRegistry::new();
    registry.register(Arc::new(FakeAdapter {
        kind,
        mode,
        requests: Arc::clone(&requests),
    }));
    let state = Arc::new(
        AppState::from_parts(store, Arc::new(registry), GenerationManager::new()).unwrap(),
    );
    (dir, state, requests)
}

fn configure(state: &AppState) -> Uuid {
    configure_protocol(state, "openai_chat_completions")
}

fn configure_protocol(state: &AppState, protocol: &str) -> Uuid {
    state
        .providers
        .upsert(ProviderInput {
            schema_version: IPC_SCHEMA_VERSION,
            id: None,
            name: "Fake".into(),
            protocol: protocol.into(),
            base_url: "https://fake.example/v1".into(),
            api_key: ApiKeyUpdateInput::Replace {
                value: "sk-secret-test-key".into(),
            },
            models: vec![ModelRecord {
                id: "fake-model".into(),
                display_name: "Fake Model".into(),
                source: ModelSource::Manual,
                temperature: Some(0.2),
                max_tokens: Some(100),
            }],
        })
        .unwrap()
        .id
}

#[tokio::test]
async fn success_stream_persists_user_and_assistant() {
    let (_dir, state, requests) = test_state(FakeMode::Success);
    let provider_id = configure(&state);
    let session = state.sessions.create(None).unwrap();
    let sink = CaptureSink::default();

    let assistant = state
        .chat
        .start_chat(
            ChatInput {
                schema_version: IPC_SCHEMA_VERSION,
                session_id: session.id,
                provider_id,
                model_id: "fake-model".into(),
                content: "question".into(),
            },
            &sink,
        )
        .await
        .unwrap();

    assert_eq!(assistant.status, Some(AssistantStatus::Completed));
    assert_eq!(assistant.content, "hello");
    assert_eq!(assistant.reasoning.as_deref(), Some("think"));
    let loaded = state.sessions.load(session.id).unwrap();
    assert_eq!(loaded.messages.len(), 2);
    assert_eq!(loaded.messages[0].content, "question");
    assert_eq!(loaded.messages[1].id, assistant.id);

    let events = sink.events.lock().unwrap();
    assert_eq!(events[0].sequence, 0);
    assert!(events
        .windows(2)
        .all(|window| window[1].sequence == window[0].sequence + 1));
    assert!(events
        .iter()
        .all(|event| event.assistant_message_id == assistant.id));

    let request = requests.lock().unwrap()[0].clone();
    assert_eq!(request.messages.last().unwrap().text_content(), "question");
    assert_eq!(request.temperature, Some(0.2));
}

#[tokio::test]
async fn chat_dispatches_to_registered_responses_adapter() {
    let (_dir, state, requests) =
        test_state_for_protocol(FakeMode::Success, ProtocolKind::OpenAiResponses);
    let provider_id = configure_protocol(&state, "openai_responses");
    let session = state.sessions.create(None).unwrap();

    let assistant = state
        .chat
        .start_chat(
            ChatInput {
                schema_version: IPC_SCHEMA_VERSION,
                session_id: session.id,
                provider_id,
                model_id: "fake-model".into(),
                content: "question".into(),
            },
            &CaptureSink::default(),
        )
        .await
        .unwrap();

    assert_eq!(assistant.status, Some(AssistantStatus::Completed));
    assert_eq!(assistant.content, "hello");
    assert_eq!(requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn chat_dispatches_to_registered_anthropic_adapter() {
    let (_dir, state, requests) =
        test_state_for_protocol(FakeMode::Success, ProtocolKind::AnthropicMessages);
    let provider_id = configure_protocol(&state, "anthropic_messages");
    let session = state.sessions.create(None).unwrap();

    let assistant = state
        .chat
        .start_chat(
            ChatInput {
                schema_version: IPC_SCHEMA_VERSION,
                session_id: session.id,
                provider_id,
                model_id: "fake-model".into(),
                content: "question".into(),
            },
            &CaptureSink::default(),
        )
        .await
        .unwrap();

    assert_eq!(assistant.status, Some(AssistantStatus::Completed));
    assert_eq!(assistant.content, "hello");
    assert_eq!(requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn chat_dispatches_to_registered_gemini_adapter() {
    let (_dir, state, requests) =
        test_state_for_protocol(FakeMode::Success, ProtocolKind::GeminiGenerateContent);
    let provider_id = configure_protocol(&state, "gemini_generate_content");
    let session = state.sessions.create(None).unwrap();

    let assistant = state
        .chat
        .start_chat(
            ChatInput {
                schema_version: IPC_SCHEMA_VERSION,
                session_id: session.id,
                provider_id,
                model_id: "fake-model".into(),
                content: "question".into(),
            },
            &CaptureSink::default(),
        )
        .await
        .unwrap();

    assert_eq!(assistant.status, Some(AssistantStatus::Completed));
    assert_eq!(assistant.content, "hello");
    assert_eq!(requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn immediate_adapter_failure_keeps_user_and_persists_error() {
    let (_dir, state, _requests) = test_state(FakeMode::Fail);
    let provider_id = configure(&state);
    let session = state.sessions.create(None).unwrap();
    let sink = CaptureSink::default();

    let assistant = state
        .chat
        .start_chat(
            ChatInput {
                schema_version: IPC_SCHEMA_VERSION,
                session_id: session.id,
                provider_id,
                model_id: "fake-model".into(),
                content: "persist me".into(),
            },
            &sink,
        )
        .await
        .unwrap();

    assert_eq!(assistant.status, Some(AssistantStatus::Error));
    assert_eq!(
        assistant.error.as_ref().unwrap().code,
        "provider_unavailable"
    );
    let messages = state.sessions.load(session.id).unwrap().messages;
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].content, "persist me");
}

#[test]
fn context_excludes_reasoning_usage_and_error_assistant() {
    let (_dir, state, _requests) = test_state(FakeMode::Success);
    let session = state.sessions.create(None).unwrap();
    state
        .sessions
        .update(
            session.id,
            mprism_desktop_lib::application::UpdateSessionInput {
                schema_version: IPC_SCHEMA_VERSION,
                title: None,
                system_prompt: Some("system".into()),
                set_last_provider_id: false,
                last_provider_id: None,
                set_last_model_id: false,
                last_model_id: None,
            },
        )
        .unwrap();
    let user = MessageRecord::new_user(session.id, 0, "user", state.store.device_id()).unwrap();
    state.store.append_message_and_touch(user, false).unwrap();
    let snapshots = || {
        (
            ProviderSnapshot {
                id: Uuid::now_v7(),
                name: "P".into(),
            },
            ModelSnapshot {
                id: "m".into(),
                display_name: "M".into(),
            },
        )
    };
    let (provider, model) = snapshots();
    let completed = MessageRecord::new_assistant(
        session.id,
        0,
        "answer",
        Some("private reasoning".into()),
        AssistantStatus::Completed,
        Uuid::now_v7(),
        provider,
        model,
        None,
        None,
        None,
        state.store.device_id(),
    );
    state
        .store
        .append_message_and_touch(completed, false)
        .unwrap();
    let (provider, model) = snapshots();
    let failed = MessageRecord::new_assistant(
        session.id,
        0,
        "partial error",
        None,
        AssistantStatus::Error,
        Uuid::now_v7(),
        provider,
        model,
        None,
        None,
        Some(MessageErrorRecord {
            code: "transport".into(),
            message: "safe".into(),
            retryable: true,
            http_status: None,
            provider_request_id: None,
        }),
        state.store.device_id(),
    );
    state.store.append_message_and_touch(failed, false).unwrap();

    let context = state.sessions.build_context(session.id).unwrap();
    let contents: Vec<_> = context
        .iter()
        .map(|message| message.text_content())
        .collect();
    assert_eq!(contents, vec!["system", "user", "answer"]);
}

#[tokio::test]
async fn same_session_conflicts_and_cancel_is_idempotent() {
    let notify = Arc::new(Notify::new());
    let (_dir, state, _requests) = test_state(FakeMode::Blocking(Arc::clone(&notify)));
    let provider_id = configure(&state);
    let session = state.sessions.create(None).unwrap();
    let first_sink = Arc::new(CaptureSink::default());
    let first_state = Arc::clone(&state);
    let first_sink_task = Arc::clone(&first_sink);
    let first = tokio::spawn(async move {
        first_state
            .chat
            .start_chat(
                ChatInput {
                    schema_version: IPC_SCHEMA_VERSION,
                    session_id: session.id,
                    provider_id,
                    model_id: "fake-model".into(),
                    content: "first".into(),
                },
                first_sink_task.as_ref(),
            )
            .await
    });

    for _ in 0..50 {
        if state.generations.running_count() == 1 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(state.generations.running_count(), 1);
    let second = state
        .chat
        .start_chat(
            ChatInput {
                schema_version: IPC_SCHEMA_VERSION,
                session_id: session.id,
                provider_id,
                model_id: "fake-model".into(),
                content: "second".into(),
            },
            &CaptureSink::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(second.code, "conflict");

    let request_id = first_sink.events.lock().unwrap()[0].request_id;
    assert!(state.chat.cancel(request_id));
    let assistant = first.await.unwrap().unwrap();
    assert_eq!(assistant.status, Some(AssistantStatus::Stopped));
    assert!(!state.chat.cancel(request_id));
    assert_eq!(state.generations.running_count(), 0);
}

#[tokio::test]
async fn different_sessions_can_generate_concurrently() {
    let notify = Arc::new(Notify::new());
    let (_dir, state, _requests) = test_state(FakeMode::Blocking(Arc::clone(&notify)));
    let provider_id = configure(&state);
    let first_session = state.sessions.create(None).unwrap();
    let second_session = state.sessions.create(None).unwrap();

    let spawn_generation = |session_id| {
        let state = Arc::clone(&state);
        let sink = Arc::new(CaptureSink::default());
        let task_sink = Arc::clone(&sink);
        let task = tokio::spawn(async move {
            state
                .chat
                .start_chat(
                    ChatInput {
                        schema_version: IPC_SCHEMA_VERSION,
                        session_id,
                        provider_id,
                        model_id: "fake-model".into(),
                        content: "go".into(),
                    },
                    task_sink.as_ref(),
                )
                .await
        });
        (sink, task)
    };
    let (sink1, task1) = spawn_generation(first_session.id);
    let (sink2, task2) = spawn_generation(second_session.id);

    for _ in 0..50 {
        if state.generations.running_count() == 2 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(state.generations.running_count(), 2);
    let request1 = sink1.events.lock().unwrap()[0].request_id;
    let request2 = sink2.events.lock().unwrap()[0].request_id;
    assert!(state.chat.cancel(request1));
    assert!(state.chat.cancel(request2));
    assert_eq!(
        task1.await.unwrap().unwrap().status,
        Some(AssistantStatus::Stopped)
    );
    assert_eq!(
        task2.await.unwrap().unwrap().status,
        Some(AssistantStatus::Stopped)
    );
}

#[test]
fn provider_public_serialization_has_no_api_key() {
    let (_dir, state, _requests) = test_state(FakeMode::Success);
    configure(&state);
    let json = serde_json::to_string(&state.providers.providers()).unwrap();
    assert!(!json.contains("sk-secret-test-key"));
    assert!(!json.contains("\"api_key\""));
    assert!(json.contains("api_key_present"));
    assert!(json.contains("openai_chat_completions"));
}
