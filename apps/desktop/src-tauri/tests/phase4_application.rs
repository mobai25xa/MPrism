use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures_util::stream;
use mprism_desktop_lib::application::{
    ApiKeyUpdateInput, ChatInput, GenerationManager, ProviderInput, ReasoningPolicyInput,
    StreamEnvelope, StreamSink, IPC_SCHEMA_VERSION,
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
    /// Emits tool call stream then Completed(ToolCalls).
    ToolCalls,
    Fail,
    Blocking(Arc<Notify>),
}

struct FakeAdapter {
    kind: ProtocolKind,
    mode: FakeMode,
    requests: Arc<Mutex<Vec<ChatRequest>>>,
    /// When true, `capabilities().tools` is false (for gate tests).
    disable_tools: bool,
}

#[async_trait]
impl ProtocolAdapter for FakeAdapter {
    fn kind(&self) -> ProtocolKind {
        self.kind
    }

    fn capabilities(&self) -> ProtocolCapabilities {
        // Approximate matrix flags for desktop gate tests (not full production adapters).
        let tools = !self.disable_tools;
        match self.kind {
            ProtocolKind::OpenAiChatCompletions => ProtocolCapabilities {
                streaming: true,
                list_models: true,
                reasoning_output: true,
                reasoning_control: false,
                tools,
                // Fake: treat Chat Completions as no-vision for desktop gate tests.
                vision_input: false,
                stream_usage: true,
                custom_headers: true,
                api_key_query: true,
            },
            _ => ProtocolCapabilities {
                streaming: true,
                list_models: true,
                reasoning_output: true,
                reasoning_control: true,
                tools,
                vision_input: true,
                stream_usage: true,
                custom_headers: true,
                api_key_query: true,
            },
        }
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
            FakeMode::ToolCalls => Ok(Box::pin(stream::iter(vec![
                Ok(StreamEvent::ToolCallDelta {
                    id: Some("call_1".into()),
                    name: Some("lookup".into()),
                    arguments_delta: r#"{"q":"#.into(),
                    index: Some(0),
                }),
                Ok(StreamEvent::ToolCallDelta {
                    id: Some("call_1".into()),
                    name: None,
                    arguments_delta: r#"1"}"#.into(),
                    index: Some(0),
                }),
                Ok(StreamEvent::ToolCallFinished {
                    id: "call_1".into(),
                    name: "lookup".into(),
                    arguments: r#"{"q":1}"#.into(),
                    index: Some(0),
                }),
                Ok(StreamEvent::Usage(TokenUsage {
                    prompt_tokens: Some(4),
                    completion_tokens: Some(1),
                    total_tokens: Some(5),
                    reasoning_tokens: Some(2),
                    cached_tokens: Some(1),
                })),
                Ok(StreamEvent::Completed {
                    finish_reason: FinishReason::ToolCalls,
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
        disable_tools: false,
    }));
    let state = Arc::new(
        AppState::from_parts(store, Arc::new(registry), GenerationManager::new()).unwrap(),
    );
    (dir, state, requests)
}

fn test_state_no_tools(
    mode: FakeMode,
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
        kind: ProtocolKind::OpenAiChatCompletions,
        mode,
        requests: Arc::clone(&requests),
        disable_tools: true,
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
                reasoning: None,
            }],
            tools: vec![],
            tool_choice: None,
            extra_headers: vec![],
            api_key_query_param: None,
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
                reasoning: None,
                attachments: None,
                tools: None,
                tool_choice: None,
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
                reasoning: None,
                attachments: None,
                tools: None,
                tool_choice: None,
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
                reasoning: None,
                attachments: None,
                tools: None,
                tool_choice: None,
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
                reasoning: None,
                attachments: None,
                tools: None,
                tool_choice: None,
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
                reasoning: None,
                attachments: None,
                tools: None,
                tool_choice: None,
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
        vec![],
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
        vec![],
        Some(MessageErrorRecord {
            code: "transport".into(),
            message: "safe".into(),
            retryable: true,
            http_status: None,
            provider_request_id: None,
            retry_after_ms: None,
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
                    reasoning: None,
                    attachments: None,
                    tools: None,
                    tool_choice: None,
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
                reasoning: None,
                attachments: None,
                tools: None,
                tool_choice: None,
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
                        reasoning: None,
                        attachments: None,
                        tools: None,
                        tool_choice: None,
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

#[tokio::test]
async fn reasoning_on_rejected_when_protocol_lacks_control_before_persist() {
    let (_dir, state, requests) = test_state(FakeMode::Success);
    let provider_id = configure(&state);
    let session = state.sessions.create(None).unwrap();
    let err = state
        .chat
        .start_chat(
            ChatInput {
                schema_version: IPC_SCHEMA_VERSION,
                session_id: session.id,
                provider_id,
                model_id: "fake-model".into(),
                content: "question".into(),
                reasoning: Some(ReasoningPolicyInput {
                    mode: "on".into(),
                    effort: Some("low".into()),
                    budget_tokens: None,
                }),
                attachments: None,
                tools: None,
                tool_choice: None,
            },
            &CaptureSink::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, "unsupported");
    assert!(requests.lock().unwrap().is_empty());
    assert!(state.sessions.load(session.id).unwrap().messages.is_empty());
}

#[tokio::test]
async fn reasoning_on_accepted_for_responses_protocol() {
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
                reasoning: Some(ReasoningPolicyInput {
                    mode: "on".into(),
                    effort: Some("medium".into()),
                    budget_tokens: None,
                }),
                attachments: None,
                tools: None,
                tool_choice: None,
            },
            &CaptureSink::default(),
        )
        .await
        .unwrap();
    assert_eq!(assistant.status, Some(AssistantStatus::Completed));
    let req = requests.lock().unwrap()[0].clone();
    let policy = req.reasoning.expect("reasoning policy");
    assert!(matches!(policy.mode, mprism_protocol::ReasoningMode::On));
    assert_eq!(
        policy.effort,
        Some(mprism_protocol::ReasoningEffort::Medium)
    );
}

#[tokio::test]
async fn model_stored_reasoning_applied_when_chat_input_omits_policy() {
    use mprism_desktop_lib::storage::StoredReasoningSettings;

    let (_dir, state, requests) =
        test_state_for_protocol(FakeMode::Success, ProtocolKind::OpenAiResponses);
    let provider_id = state
        .providers
        .upsert(ProviderInput {
            schema_version: IPC_SCHEMA_VERSION,
            id: None,
            name: "Fake".into(),
            protocol: "openai_responses".into(),
            base_url: "https://fake.example/v1".into(),
            api_key: ApiKeyUpdateInput::Replace {
                value: "sk-secret-test-key".into(),
            },
            models: vec![ModelRecord {
                id: "fake-model".into(),
                display_name: "Fake Model".into(),
                source: ModelSource::Manual,
                temperature: None,
                max_tokens: None,
                reasoning: Some(StoredReasoningSettings {
                    mode: "on".into(),
                    effort: Some("high".into()),
                    budget_tokens: None,
                }),
            }],
            tools: vec![],
            tool_choice: None,
            extra_headers: vec![],
            api_key_query_param: None,
        })
        .unwrap()
        .id;
    let session = state.sessions.create(None).unwrap();
    state
        .chat
        .start_chat(
            ChatInput {
                schema_version: IPC_SCHEMA_VERSION,
                session_id: session.id,
                provider_id,
                model_id: "fake-model".into(),
                content: "question".into(),
                reasoning: None,
                attachments: None,
                tools: None,
                tool_choice: None,
            },
            &CaptureSink::default(),
        )
        .await
        .unwrap();
    let req = requests.lock().unwrap()[0].clone();
    let policy = req.reasoning.expect("stored model reasoning");
    assert!(matches!(policy.mode, mprism_protocol::ReasoningMode::On));
    assert_eq!(policy.effort, Some(mprism_protocol::ReasoningEffort::High));
}

#[tokio::test]
async fn default_unconfigured_reasoning_is_none_on_request() {
    let (_dir, state, requests) =
        test_state_for_protocol(FakeMode::Success, ProtocolKind::OpenAiResponses);
    let provider_id = configure_protocol(&state, "openai_responses");
    let session = state.sessions.create(None).unwrap();
    state
        .chat
        .start_chat(
            ChatInput {
                schema_version: IPC_SCHEMA_VERSION,
                session_id: session.id,
                provider_id,
                model_id: "fake-model".into(),
                content: "question".into(),
                reasoning: None,
                attachments: None,
                tools: None,
                tool_choice: None,
            },
            &CaptureSink::default(),
        )
        .await
        .unwrap();
    assert!(requests.lock().unwrap()[0].reasoning.is_none());
}

#[tokio::test]
async fn attachments_rejected_when_protocol_lacks_vision_before_persist() {
    let (_dir, state, requests) = test_state(FakeMode::Success);
    let provider_id = configure(&state);
    let session = state.sessions.create(None).unwrap();
    let imported = state
        .store
        .import_attachment(&[0x89, b'P', b'N', b'G', 1, 2, 3, 4], "image/png", None)
        .unwrap();
    let err = state
        .chat
        .start_chat(
            ChatInput {
                schema_version: IPC_SCHEMA_VERSION,
                session_id: session.id,
                provider_id,
                model_id: "fake-model".into(),
                content: "see".into(),
                reasoning: None,
                attachments: Some(vec![mprism_desktop_lib::application::ChatAttachmentInput {
                    id: imported.id.to_string(),
                    media_type: Some("image/png".into()),
                }]),
                tools: None,
                tool_choice: None,
            },
            &CaptureSink::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, "unsupported");
    assert!(requests.lock().unwrap().is_empty());
    assert!(state.sessions.load(session.id).unwrap().messages.is_empty());
}

#[tokio::test]
async fn attachments_build_image_base64_parts_for_vision_protocol() {
    use mprism_protocol::ContentPart;

    let (_dir, state, requests) =
        test_state_for_protocol(FakeMode::Success, ProtocolKind::OpenAiResponses);
    let provider_id = configure_protocol(&state, "openai_responses");
    let session = state.sessions.create(None).unwrap();
    let png = [0x89u8, b'P', b'N', b'G', 9, 8, 7, 6];
    let imported = state
        .store
        .import_attachment(&png, "image/png", Some("shot.png".into()))
        .unwrap();

    let assistant = state
        .chat
        .start_chat(
            ChatInput {
                schema_version: IPC_SCHEMA_VERSION,
                session_id: session.id,
                provider_id,
                model_id: "fake-model".into(),
                content: "what is this".into(),
                reasoning: None,
                attachments: Some(vec![mprism_desktop_lib::application::ChatAttachmentInput {
                    id: imported.id.to_string(),
                    media_type: Some("image/png".into()),
                }]),
                tools: None,
                tool_choice: None,
            },
            &CaptureSink::default(),
        )
        .await
        .unwrap();
    assert_eq!(assistant.status, Some(AssistantStatus::Completed));

    let loaded = state.sessions.load(session.id).unwrap();
    assert_eq!(loaded.messages[0].attachments.len(), 1);
    assert_eq!(loaded.messages[0].attachments[0].attachment_id, imported.id);
    // Must not persist base64 body in JSONL message content path.
    let raw = std::fs::read_to_string(
        state
            .store
            .root()
            .join("sessions")
            .join(session.id.to_string())
            .join("messages.jsonl"),
    )
    .unwrap();
    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
    assert!(!raw.contains(&B64.encode(png)));

    let req = requests.lock().unwrap()[0].clone();
    let user = req
        .messages
        .iter()
        .find(|m| m.role == mprism_protocol::ChatRole::User)
        .expect("user message");
    assert!(user
        .parts
        .iter()
        .any(|p| matches!(p, ContentPart::Text { .. })));
    assert!(user.parts.iter().any(
        |p| matches!(p, ContentPart::ImageBase64 { media_type, .. } if media_type == "image/png")
    ));
}

#[test]
fn list_protocol_capabilities_for_registered_adapters() {
    let (_dir, state, _requests) = test_state(FakeMode::Success);
    let caps = state.providers.list_protocol_capabilities();
    assert_eq!(caps.len(), 1);
    assert_eq!(caps[0].protocol, "openai_chat_completions");
    assert!(!caps[0].reasoning_control);
    assert!(caps[0].tools);

    let one = state
        .providers
        .protocol_capabilities("openai_chat_completions")
        .unwrap();
    assert!(!one.reasoning_control);
    assert!(state
        .providers
        .protocol_capabilities("not_a_protocol")
        .is_err());
}

#[tokio::test]
async fn tool_call_stream_persists_calls_and_stable_finish_reason() {
    use mprism_desktop_lib::application::StreamEventPayload;

    let (_dir, state, _requests) = test_state(FakeMode::ToolCalls);
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
                content: "use tool".into(),
                reasoning: None,
                attachments: None,
                tools: None,
                tool_choice: None,
            },
            &sink,
        )
        .await
        .unwrap();

    assert_eq!(assistant.status, Some(AssistantStatus::Completed));
    assert_eq!(assistant.finish_reason.as_deref(), Some("tool_calls"));
    assert_eq!(assistant.tool_calls.len(), 1);
    assert_eq!(assistant.tool_calls[0].id, "call_1");
    assert_eq!(assistant.tool_calls[0].name, "lookup");
    assert_eq!(assistant.tool_calls[0].arguments, r#"{"q":1}"#);
    assert_eq!(
        assistant.usage.as_ref().and_then(|u| u.reasoning_tokens),
        Some(2)
    );
    assert_eq!(
        assistant.usage.as_ref().and_then(|u| u.cached_tokens),
        Some(1)
    );

    let events = sink.events.lock().unwrap();
    assert!(events
        .iter()
        .any(|e| matches!(e.event, StreamEventPayload::ToolCallDelta { .. })));
    assert!(events
        .iter()
        .any(|e| matches!(e.event, StreamEventPayload::ToolCallFinished { .. })));
    assert!(events.iter().any(|e| matches!(
        &e.event,
        StreamEventPayload::Completed {
            finish_reason: Some(reason)
        } if reason == "tool_calls"
    )));
}

#[tokio::test]
async fn cancelled_stream_has_stopped_status_without_success_finish_reason() {
    let notify = Arc::new(Notify::new());
    let (_dir, state, _requests) = test_state(FakeMode::Blocking(Arc::clone(&notify)));
    let provider_id = configure(&state);
    let session = state.sessions.create(None).unwrap();
    let sink = Arc::new(CaptureSink::default());
    let task_sink = Arc::clone(&sink);
    let state_clone = Arc::clone(&state);
    let task = tokio::spawn(async move {
        state_clone
            .chat
            .start_chat(
                ChatInput {
                    schema_version: IPC_SCHEMA_VERSION,
                    session_id: session.id,
                    provider_id,
                    model_id: "fake-model".into(),
                    content: "go".into(),
                    reasoning: None,
                    attachments: None,
                    tools: None,
                    tool_choice: None,
                },
                task_sink.as_ref(),
            )
            .await
    });

    for _ in 0..50 {
        if state.generations.running_count() == 1 {
            break;
        }
        tokio::task::yield_now().await;
    }
    let request_id = sink.events.lock().unwrap()[0].request_id;
    assert!(state.chat.cancel(request_id));
    let assistant = task.await.unwrap().unwrap();
    assert_eq!(assistant.status, Some(AssistantStatus::Stopped));
    assert!(assistant.finish_reason.is_none());
    let has_stopped = sink.events.lock().unwrap().iter().any(|e| {
        matches!(
            e.event,
            mprism_desktop_lib::application::StreamEventPayload::Stopped
        )
    });
    assert!(has_stopped);
    let has_completed = sink.events.lock().unwrap().iter().any(|e| {
        matches!(
            e.event,
            mprism_desktop_lib::application::StreamEventPayload::Completed { .. }
        )
    });
    assert!(!has_completed);
}

#[tokio::test]
async fn provider_tools_applied_to_chat_request() {
    use mprism_desktop_lib::storage::{StoredToolChoice, StoredToolDefinition};
    use mprism_protocol::ToolChoice;

    let (_dir, state, requests) = test_state(FakeMode::Success);
    let provider_id = state
        .providers
        .upsert(ProviderInput {
            schema_version: IPC_SCHEMA_VERSION,
            id: None,
            name: "Fake".into(),
            protocol: "openai_chat_completions".into(),
            base_url: "https://fake.example/v1".into(),
            api_key: ApiKeyUpdateInput::Replace {
                value: "sk-secret-test-key".into(),
            },
            models: vec![ModelRecord {
                id: "fake-model".into(),
                display_name: "Fake Model".into(),
                source: ModelSource::Manual,
                temperature: None,
                max_tokens: None,
                reasoning: None,
            }],
            tools: vec![StoredToolDefinition {
                name: "lookup".into(),
                description: Some("look up".into()),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": { "q": { "type": "string" } }
                }),
            }],
            tool_choice: Some(StoredToolChoice {
                mode: "auto".into(),
                name: None,
            }),
            extra_headers: vec![],
            api_key_query_param: None,
        })
        .unwrap()
        .id;
    let session = state.sessions.create(None).unwrap();
    state
        .chat
        .start_chat(
            ChatInput {
                schema_version: IPC_SCHEMA_VERSION,
                session_id: session.id,
                provider_id,
                model_id: "fake-model".into(),
                content: "use tool".into(),
                reasoning: None,
                attachments: None,
                tools: None,
                tool_choice: None,
            },
            &CaptureSink::default(),
        )
        .await
        .unwrap();
    let req = requests.lock().unwrap()[0].clone();
    let tools = req.tools.expect("tools present");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "lookup");
    assert!(matches!(req.tool_choice, Some(ToolChoice::Auto)));
}

#[tokio::test]
async fn no_tools_means_none_on_request() {
    let (_dir, state, requests) = test_state(FakeMode::Success);
    let provider_id = configure(&state);
    let session = state.sessions.create(None).unwrap();
    state
        .chat
        .start_chat(
            ChatInput {
                schema_version: IPC_SCHEMA_VERSION,
                session_id: session.id,
                provider_id,
                model_id: "fake-model".into(),
                content: "plain".into(),
                reasoning: None,
                attachments: None,
                tools: None,
                tool_choice: None,
            },
            &CaptureSink::default(),
        )
        .await
        .unwrap();
    let req = requests.lock().unwrap()[0].clone();
    assert!(req.tools.is_none());
    assert!(req.tool_choice.is_none());
}

#[tokio::test]
async fn tools_rejected_when_protocol_lacks_tools_before_persist() {
    use mprism_desktop_lib::application::ToolDefinitionInput;

    let (_dir, state, requests) = test_state_no_tools(FakeMode::Success);
    let provider_id = configure(&state);
    let session = state.sessions.create(None).unwrap();
    let err = state
        .chat
        .start_chat(
            ChatInput {
                schema_version: IPC_SCHEMA_VERSION,
                session_id: session.id,
                provider_id,
                model_id: "fake-model".into(),
                content: "tool me".into(),
                reasoning: None,
                attachments: None,
                tools: Some(vec![ToolDefinitionInput {
                    name: "lookup".into(),
                    description: None,
                    parameters: serde_json::json!({ "type": "object" }),
                }]),
                tool_choice: None,
            },
            &CaptureSink::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, "unsupported");
    assert!(requests.lock().unwrap().is_empty());
    assert!(state.sessions.load(session.id).unwrap().messages.is_empty());
}

#[tokio::test]
async fn anthropic_thinking_on_rejects_required_tool_choice_before_persist() {
    use mprism_desktop_lib::application::{ToolChoiceInput, ToolDefinitionInput};

    let (_dir, state, requests) =
        test_state_for_protocol(FakeMode::Success, ProtocolKind::AnthropicMessages);
    let provider_id = configure_protocol(&state, "anthropic_messages");
    let session = state.sessions.create(None).unwrap();
    let err = state
        .chat
        .start_chat(
            ChatInput {
                schema_version: IPC_SCHEMA_VERSION,
                session_id: session.id,
                provider_id,
                model_id: "fake-model".into(),
                content: "think and tool".into(),
                reasoning: Some(ReasoningPolicyInput {
                    mode: "on".into(),
                    effort: None,
                    budget_tokens: Some(1024),
                }),
                attachments: None,
                tools: Some(vec![ToolDefinitionInput {
                    name: "lookup".into(),
                    description: None,
                    parameters: serde_json::json!({ "type": "object" }),
                }]),
                tool_choice: Some(ToolChoiceInput {
                    mode: "required".into(),
                    name: None,
                }),
            },
            &CaptureSink::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, "validation");
    assert!(err.message.contains("tool_choice"));
    assert!(requests.lock().unwrap().is_empty());
    assert!(state.sessions.load(session.id).unwrap().messages.is_empty());
}

#[tokio::test]
async fn auth_options_applied_to_chat_endpoint() {
    use mprism_desktop_lib::storage::StoredExtraHeader;

    let (_dir, state, requests) = test_state(FakeMode::Success);
    let provider_id = state
        .providers
        .upsert(ProviderInput {
            schema_version: IPC_SCHEMA_VERSION,
            id: None,
            name: "Fake".into(),
            protocol: "openai_chat_completions".into(),
            base_url: "https://fake.example/v1".into(),
            api_key: ApiKeyUpdateInput::Replace {
                value: "sk-secret-test-key".into(),
            },
            models: vec![ModelRecord {
                id: "fake-model".into(),
                display_name: "Fake Model".into(),
                source: ModelSource::Manual,
                temperature: None,
                max_tokens: None,
                reasoning: None,
            }],
            tools: vec![],
            tool_choice: None,
            extra_headers: vec![StoredExtraHeader {
                name: "X-Custom".into(),
                value: "yes".into(),
            }],
            api_key_query_param: Some("key".into()),
        })
        .unwrap()
        .id;
    // Capture endpoint auth via a one-shot adapter registered? FakeAdapter only stores ChatRequest.
    // Instead inspect settings + rebuild with stored_auth_options/build_provider_endpoint.
    let settings = state.store.load_settings().unwrap();
    let provider = settings
        .providers
        .iter()
        .find(|p| p.id == provider_id)
        .unwrap();
    assert_eq!(provider.extra_headers.len(), 1);
    assert_eq!(provider.extra_headers[0].name, "X-Custom");
    assert_eq!(provider.api_key_query_param.as_deref(), Some("key"));

    let auth = mprism_desktop_lib::application::stored_auth_options(
        &provider.extra_headers,
        provider.api_key_query_param.as_deref(),
    );
    let endpoint = mprism_desktop_lib::application::build_provider_endpoint(
        ProtocolKind::OpenAiChatCompletions,
        &provider.base_url,
        mprism_protocol::SecretString::new(provider.api_key.clone()),
        auth,
    )
    .unwrap();
    assert_eq!(endpoint.auth.extra_headers.len(), 1);
    assert_eq!(endpoint.auth.extra_headers[0].0, "X-Custom");
    assert_eq!(endpoint.auth.extra_headers[0].1, "yes");
    assert_eq!(endpoint.auth.api_key_query_param.as_deref(), Some("key"));

    // Chat still succeeds and does not regress request path.
    let session = state.sessions.create(None).unwrap();
    state
        .chat
        .start_chat(
            ChatInput {
                schema_version: IPC_SCHEMA_VERSION,
                session_id: session.id,
                provider_id,
                model_id: "fake-model".into(),
                content: "hi".into(),
                reasoning: None,
                attachments: None,
                tools: None,
                tool_choice: None,
            },
            &CaptureSink::default(),
        )
        .await
        .unwrap();
    assert_eq!(requests.lock().unwrap().len(), 1);

    // Debug/public must not print header values as free text in Debug of settings.
    let dbg = format!("{:?}", provider.extra_headers);
    assert!(dbg.contains("***"));
    assert!(!dbg.contains("yes"));
}
