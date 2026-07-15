use std::sync::Arc;

use futures_util::StreamExt;
use mprism_protocol::{
    ChatRequest, ProtocolAdapter, ProviderEndpoint, ReasoningEffort, ReasoningMode,
    ReasoningPolicy, SecretString, StreamEvent, TokenUsage, ToolChoice, ToolDefinition,
};
use parking_lot::RwLock;
use uuid::Uuid;

use crate::state::AdapterRegistry;
use crate::storage::{
    AssistantStatus, FileStore, MessageAttachmentRef, MessageRecord, ModelSnapshot,
    ProviderSnapshot, SettingsDocument, StoredToolCall, TokenUsageRecord,
};

use super::{
    build_provider_endpoint, check_ipc_schema, protocol_kind, stored_auth_options, AppError,
    ChatInput, GenerationManager, ReasoningPolicyInput, SessionService, StreamEnvelope,
    StreamEventPayload, ToolChoiceInput, ToolDefinitionInput,
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
    /// Model-level stored reasoning (used when ChatInput.reasoning is omitted).
    stored_reasoning: Option<crate::storage::StoredReasoningSettings>,
    /// Provider-level tools (used when ChatInput.tools is omitted).
    stored_tools: Vec<crate::storage::StoredToolDefinition>,
    stored_tool_choice: Option<crate::storage::StoredToolChoice>,
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
        // Per-request ChatInput wins; otherwise model settings; auto/absent → None (V1).
        let reasoning = resolve_request_reasoning(
            input.reasoning.as_ref(),
            selected.stored_reasoning.as_ref(),
        )?;
        let (tools, tool_choice) = resolve_request_tools(
            input.tools.as_ref(),
            input.tool_choice.as_ref(),
            &selected.stored_tools,
            selected.stored_tool_choice.as_ref(),
        )?;
        // Anthropic: extended thinking On + Required/Named is InvalidRequest (mapping §4).
        check_anthropic_thinking_tool_choice(
            selected.endpoint.protocol,
            reasoning.as_ref(),
            tool_choice.as_ref(),
        )?;
        let attachment_refs =
            resolve_attachment_refs(self.store.as_ref(), input.attachments.as_ref())?;

        if input.content.trim().is_empty() && attachment_refs.is_empty() {
            return Err(AppError::validation("消息内容不能为空"));
        }

        // Pre-check capabilities (vision + reasoning + tools) before mutating history.
        let preflight_user = build_preflight_user_message(&input.content, &attachment_refs)?;
        let preflight = ChatRequest {
            model: input.model_id.clone(),
            messages: vec![preflight_user],
            temperature: selected.temperature,
            max_tokens: selected.max_tokens,
            reasoning: reasoning.clone(),
            tools: tools.clone(),
            tool_choice: tool_choice.clone(),
        };
        preflight
            .check_capabilities(&selected.adapter.capabilities())
            .map_err(AppError::from)?;

        let request_id = Uuid::now_v7();
        let assistant_message_id = Uuid::now_v7();
        let (_guard, cancellation) = self.generations.register(input.session_id, request_id)?;

        let user = MessageRecord::new_user_with_attachments(
            input.session_id,
            0,
            input.content,
            attachment_refs,
            self.store.device_id(),
        )?;
        self.store.append_message_and_touch(user, true)?;

        let context = self.sessions.build_context(input.session_id)?;
        let request = ChatRequest {
            model: input.model_id,
            messages: context,
            temperature: selected.temperature,
            max_tokens: selected.max_tokens,
            reasoning,
            tools,
            tool_choice,
        };
        request.validate()?;
        request
            .check_capabilities(&selected.adapter.capabilities())
            .map_err(AppError::from)?;

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
        let mut tool_calls: Vec<StoredToolCall> = Vec::new();
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
                        Some(Ok(StreamEvent::ToolCallDelta {
                            id,
                            name,
                            arguments_delta,
                            index,
                        })) => {
                            if send_event(
                                sink,
                                request_id,
                                input.session_id,
                                assistant_message_id,
                                &mut sequence,
                                StreamEventPayload::ToolCallDelta {
                                    id,
                                    name,
                                    arguments_delta,
                                    index,
                                },
                            )
                            .is_err()
                            {
                                cancellation.cancel();
                            }
                        }
                        Some(Ok(StreamEvent::ToolCallFinished {
                            id,
                            name,
                            arguments,
                            index,
                        })) => {
                            upsert_finished_tool_call(
                                &mut tool_calls,
                                StoredToolCall {
                                    id: id.clone(),
                                    name: name.clone(),
                                    arguments: arguments.clone(),
                                    index,
                                },
                            );
                            if send_event(
                                sink,
                                request_id,
                                input.session_id,
                                assistant_message_id,
                                &mut sequence,
                                StreamEventPayload::ToolCallFinished {
                                    id,
                                    name,
                                    arguments,
                                    index,
                                },
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
                            let value_str = Some(ipc_finish_reason(&value));
                            finish_reason = value_str.clone();
                            let _ = send_event(
                                sink,
                                request_id,
                                input.session_id,
                                assistant_message_id,
                                &mut sequence,
                                StreamEventPayload::Completed {
                                    finish_reason: value_str,
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

        // Cancelled streams must not look like a successful stop finish_reason.
        let persisted_finish_reason = match final_status {
            AssistantStatus::Completed => finish_reason,
            AssistantStatus::Stopped | AssistantStatus::Error => None,
        };

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
            persisted_finish_reason,
            tool_calls,
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
        let auth = stored_auth_options(
            &provider.extra_headers,
            provider.api_key_query_param.as_deref(),
        );
        let endpoint = build_provider_endpoint(
            kind,
            &provider.base_url,
            SecretString::new(provider.api_key.clone()),
            auth,
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
            stored_reasoning: model.reasoning.clone(),
            stored_tools: provider.tools.clone(),
            stored_tool_choice: provider.tool_choice.clone(),
        })
    }
}

fn usage_record(usage: TokenUsage) -> TokenUsageRecord {
    TokenUsageRecord {
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        total_tokens: usage.total_tokens,
        reasoning_tokens: usage.reasoning_tokens,
        cached_tokens: usage.cached_tokens,
    }
}

/// Stable IPC / storage finish reason: `stop` | `length` | `content_filter` | `tool_calls` | `other:...`.
fn ipc_finish_reason(reason: &mprism_protocol::FinishReason) -> String {
    match reason {
        mprism_protocol::FinishReason::Stop => "stop".into(),
        mprism_protocol::FinishReason::Length => "length".into(),
        mprism_protocol::FinishReason::ContentFilter => "content_filter".into(),
        mprism_protocol::FinishReason::ToolCalls => "tool_calls".into(),
        mprism_protocol::FinishReason::Other(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                "other:unknown".into()
            } else if trimmed.starts_with("other:") {
                trimmed.to_string()
            } else {
                format!("other:{trimmed}")
            }
        }
    }
}

fn upsert_finished_tool_call(calls: &mut Vec<StoredToolCall>, call: StoredToolCall) {
    if let Some(idx) = call.index {
        if let Some(existing) = calls.iter_mut().find(|c| c.index == Some(idx)) {
            *existing = call;
            return;
        }
    }
    if let Some(existing) = calls.iter_mut().find(|c| c.id == call.id) {
        *existing = call;
        return;
    }
    calls.push(call);
}

fn resolve_attachment_refs(
    store: &FileStore,
    input: Option<&Vec<super::ChatAttachmentInput>>,
) -> Result<Vec<MessageAttachmentRef>, AppError> {
    let Some(items) = input else {
        return Ok(Vec::new());
    };
    if items.len() > 8 {
        return Err(AppError::validation("单次最多附带 8 张图片"));
    }
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let id = Uuid::parse_str(item.id.trim())
            .map_err(|_| AppError::validation(format!("无效的 attachment id: {}", item.id)))?;
        let meta = store.load_attachment_meta(id)?;
        let media_type = item
            .media_type
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or(meta.media_type);
        out.push(MessageAttachmentRef {
            attachment_id: id,
            media_type: Some(media_type),
        });
    }
    Ok(out)
}

fn build_preflight_user_message(
    content: &str,
    attachments: &[MessageAttachmentRef],
) -> Result<mprism_protocol::ChatMessage, AppError> {
    use mprism_protocol::{ChatMessage, ChatRole, ContentPart};

    if attachments.is_empty() {
        return Ok(ChatMessage::text(
            ChatRole::User,
            if content.trim().is_empty() {
                " "
            } else {
                content
            },
        ));
    }
    let mut parts = Vec::new();
    if !content.trim().is_empty() {
        parts.push(ContentPart::Text {
            text: content.to_string(),
        });
    }
    // Tiny placeholder images for capability gate only (real bytes loaded in build_context).
    for attachment in attachments {
        parts.push(ContentPart::ImageBase64 {
            media_type: attachment
                .media_type
                .clone()
                .unwrap_or_else(|| "image/png".into()),
            data: "AA==".into(),
        });
    }
    Ok(ChatMessage {
        role: ChatRole::User,
        parts,
        tool_call_id: None,
        tool_calls: Vec::new(),
    })
}

/// Resolve request-side policy: ChatInput override → model settings → None.
fn resolve_request_reasoning(
    input: Option<&ReasoningPolicyInput>,
    stored: Option<&crate::storage::StoredReasoningSettings>,
) -> Result<Option<ReasoningPolicy>, AppError> {
    if let Some(input) = input {
        return parse_reasoning_input(Some(input));
    }
    let Some(stored) = stored else {
        return Ok(None);
    };
    if stored.is_effective_none() {
        return Ok(None);
    }
    parse_reasoning_input(Some(&ReasoningPolicyInput {
        mode: stored.mode.clone(),
        effort: stored.effort.clone(),
        budget_tokens: stored.budget_tokens,
    }))
}

/// Resolve tools: ChatInput override → provider settings → None (V1).
fn resolve_request_tools(
    input_tools: Option<&Vec<ToolDefinitionInput>>,
    input_choice: Option<&ToolChoiceInput>,
    stored_tools: &[crate::storage::StoredToolDefinition],
    stored_choice: Option<&crate::storage::StoredToolChoice>,
) -> Result<(Option<Vec<ToolDefinition>>, Option<ToolChoice>), AppError> {
    let tools = if let Some(items) = input_tools {
        parse_tool_definitions(items)?
    } else if stored_tools.is_empty() {
        None
    } else {
        Some(
            stored_tools
                .iter()
                .map(|t| ToolDefinition {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: t.parameters.clone(),
                })
                .collect(),
        )
    };

    let tool_choice = if let Some(choice) = input_choice {
        Some(parse_tool_choice_input(choice)?)
    } else if let Some(stored) = stored_choice {
        Some(parse_stored_tool_choice(stored)?)
    } else if tools.is_some() {
        // Default when tools present and no explicit choice: Auto.
        Some(ToolChoice::Auto)
    } else {
        None
    };

    // SDK: tools None cannot carry tool_choice.
    if tools.is_none() && tool_choice.is_some() {
        return Err(AppError::validation("未配置 tools 时不能设置 tool_choice"));
    }
    // Empty list is invalid for SDK; treat as no tools if somehow empty after parse.
    if let Some(ref list) = tools {
        if list.is_empty() {
            return Ok((None, None));
        }
    }
    Ok((tools, tool_choice))
}

fn parse_tool_definitions(
    items: &[ToolDefinitionInput],
) -> Result<Option<Vec<ToolDefinition>>, AppError> {
    if items.is_empty() {
        return Ok(None);
    }
    let mut out = Vec::with_capacity(items.len());
    let mut names = std::collections::HashSet::new();
    for item in items {
        let name = item.name.trim();
        if name.is_empty() {
            return Err(AppError::validation("tool name 不能为空"));
        }
        if !item.parameters.is_object() {
            return Err(AppError::validation("tool parameters 必须是 JSON object"));
        }
        if !names.insert(name.to_string()) {
            return Err(AppError::validation(format!("tool name 重复: {name}")));
        }
        out.push(ToolDefinition {
            name: name.to_string(),
            description: item
                .description
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            parameters: item.parameters.clone(),
        });
    }
    Ok(Some(out))
}

fn parse_tool_choice_input(input: &ToolChoiceInput) -> Result<ToolChoice, AppError> {
    match input.mode.trim().to_ascii_lowercase().as_str() {
        "auto" | "" => Ok(ToolChoice::Auto),
        "none" => Ok(ToolChoice::None),
        "required" => Ok(ToolChoice::Required),
        "named" => {
            let name = input
                .name
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| AppError::validation("tool_choice named 必须提供 name"))?;
            Ok(ToolChoice::Named {
                name: name.to_string(),
            })
        }
        other => Err(AppError::validation(format!(
            "不支持的 tool_choice.mode: {other}"
        ))),
    }
}

fn parse_stored_tool_choice(
    stored: &crate::storage::StoredToolChoice,
) -> Result<ToolChoice, AppError> {
    parse_tool_choice_input(&ToolChoiceInput {
        mode: stored.mode.clone(),
        name: stored.name.clone(),
    })
}

/// Desktop preflight for Anthropic thinking + Required/Named (SDK also rejects at encode).
fn check_anthropic_thinking_tool_choice(
    protocol: mprism_protocol::ProtocolKind,
    reasoning: Option<&ReasoningPolicy>,
    tool_choice: Option<&ToolChoice>,
) -> Result<(), AppError> {
    if protocol != mprism_protocol::ProtocolKind::AnthropicMessages {
        return Ok(());
    }
    let thinking_on = matches!(reasoning.map(|p| p.mode), Some(ReasoningMode::On));
    if !thinking_on {
        return Ok(());
    }
    match tool_choice {
        Some(ToolChoice::Required) | Some(ToolChoice::Named { .. }) => Err(AppError::validation(
            "Anthropic extended thinking 启用时 tool_choice 仅允许 Auto 或 None",
        )),
        _ => Ok(()),
    }
}

/// Map IPC reasoning to SDK policy. `None` or mode `auto` → no request control (V1-compatible).
fn parse_reasoning_input(
    input: Option<&ReasoningPolicyInput>,
) -> Result<Option<ReasoningPolicy>, AppError> {
    let Some(input) = input else {
        return Ok(None);
    };
    let mode = match input.mode.trim().to_ascii_lowercase().as_str() {
        "auto" | "" => return Ok(None),
        "off" => ReasoningMode::Off,
        "on" => ReasoningMode::On,
        other => {
            return Err(AppError::validation(format!(
                "不支持的 reasoning.mode: {other}"
            )));
        }
    };
    let effort = match input
        .effort
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        None => None,
        Some(raw) => Some(parse_reasoning_effort(raw)?),
    };
    Ok(Some(ReasoningPolicy {
        mode,
        effort,
        budget_tokens: input.budget_tokens,
    }))
}

fn parse_reasoning_effort(raw: &str) -> Result<ReasoningEffort, AppError> {
    match raw.to_ascii_lowercase().as_str() {
        "minimal" => Ok(ReasoningEffort::Minimal),
        "low" => Ok(ReasoningEffort::Low),
        "medium" => Ok(ReasoningEffort::Medium),
        "high" => Ok(ReasoningEffort::High),
        "xhigh" | "x_high" => Ok(ReasoningEffort::XHigh),
        "max" => Ok(ReasoningEffort::Max),
        other => Err(AppError::validation(format!(
            "不支持的 reasoning.effort: {other}"
        ))),
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
