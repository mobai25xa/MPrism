use std::sync::Arc;

use mprism_protocol::{
    AuthOptions, ModelInfo, ProtocolCapabilities, ProtocolKind, ProviderEndpoint, SecretString,
};
use parking_lot::RwLock;
use uuid::Uuid;

use crate::state::AdapterRegistry;
use crate::storage::{
    ApiKeyUpdate, FileStore, ProviderPublic, ProviderUpsert, SettingsDocument, StoredProtocol,
    ThemePreference,
};

use super::{
    check_ipc_schema, ApiKeyUpdateInput, AppError, ModelInfoPayload, ProtocolCapabilitiesPayload,
    ProviderDraft, ProviderInput,
};

pub struct ProviderService {
    store: Arc<FileStore>,
    settings: Arc<RwLock<SettingsDocument>>,
    adapters: Arc<AdapterRegistry>,
}

impl ProviderService {
    pub fn new(
        store: Arc<FileStore>,
        settings: Arc<RwLock<SettingsDocument>>,
        adapters: Arc<AdapterRegistry>,
    ) -> Self {
        Self {
            store,
            settings,
            adapters,
        }
    }

    pub fn providers(&self) -> Vec<ProviderPublic> {
        self.settings
            .read()
            .providers
            .iter()
            .map(ProviderPublic::from)
            .collect()
    }

    pub fn set_theme(&self, theme: ThemePreference) -> Result<ThemePreference, AppError> {
        let doc = self.store.set_theme(theme)?;
        *self.settings.write() = doc;
        Ok(theme)
    }

    pub fn upsert(&self, input: ProviderInput) -> Result<ProviderPublic, AppError> {
        check_ipc_schema(input.schema_version)?;
        let protocol = parse_protocol(&input.protocol)?;
        let key = map_key_update(input.api_key);
        let (doc, provider) = self.store.upsert_provider(ProviderUpsert {
            id: input.id,
            name: input.name,
            protocol,
            base_url: input.base_url,
            api_key: key,
            models: input.models,
            tools: input.tools,
            tool_choice: input.tool_choice,
            extra_headers: input.extra_headers,
            api_key_query_param: input.api_key_query_param,
        })?;
        *self.settings.write() = doc;
        Ok(provider)
    }

    pub fn delete(&self, provider_id: Uuid) -> Result<(), AppError> {
        let doc = self.store.delete_provider(provider_id)?;
        *self.settings.write() = doc;
        Ok(())
    }

    pub fn set_defaults(
        &self,
        provider_id: Option<Uuid>,
        model_id: Option<String>,
    ) -> Result<(), AppError> {
        let doc = self.store.set_defaults(provider_id, model_id)?;
        *self.settings.write() = doc;
        Ok(())
    }

    pub async fn discover_models(
        &self,
        draft: ProviderDraft,
    ) -> Result<Vec<ModelInfoPayload>, AppError> {
        check_ipc_schema(draft.schema_version)?;
        let endpoint = self.resolve_discovery_endpoint(draft)?;
        let adapter = self.adapters.get(endpoint.protocol)?;
        let models = adapter.list_models(&endpoint).await?;
        Ok(models.into_iter().map(ModelInfoPayload::from).collect())
    }

    /// Capabilities for all registered adapters (UI gating).
    pub fn list_protocol_capabilities(&self) -> Vec<ProtocolCapabilitiesPayload> {
        self.adapters
            .list_kinds()
            .into_iter()
            .filter_map(|kind| {
                let adapter = self.adapters.get(kind).ok()?;
                Some(capabilities_payload(kind, adapter.capabilities()))
            })
            .collect()
    }

    pub fn protocol_capabilities(
        &self,
        protocol: &str,
    ) -> Result<ProtocolCapabilitiesPayload, AppError> {
        let kind = parse_protocol_kind(protocol)?;
        let adapter = self.adapters.get(kind)?;
        Ok(capabilities_payload(kind, adapter.capabilities()))
    }

    fn resolve_discovery_endpoint(
        &self,
        draft: ProviderDraft,
    ) -> Result<ProviderEndpoint, AppError> {
        let settings = self.settings.read();
        let saved = draft
            .provider_id
            .and_then(|id| settings.providers.iter().find(|p| p.id == id));

        let protocol = match draft.protocol.as_deref() {
            Some(raw) => parse_protocol_kind(raw)?,
            None => saved
                .map(|p| protocol_kind(p.protocol))
                .ok_or_else(|| AppError::validation("缺少 protocol"))?,
        };
        let base_url = draft
            .base_url
            .filter(|v| !v.trim().is_empty())
            .or_else(|| saved.map(|p| p.base_url.clone()))
            .ok_or_else(|| AppError::validation("缺少 Base URL"))?;

        let key = match draft.api_key {
            Some(ApiKeyUpdateInput::Replace { value }) => value,
            Some(ApiKeyUpdateInput::Clear) => String::new(),
            Some(ApiKeyUpdateInput::Keep) | None => saved
                .map(|p| p.api_key.clone())
                .ok_or_else(|| AppError::validation("新服务商不能使用 keep API Key"))?,
        };
        let auth = match saved {
            Some(p) => stored_auth_options(&p.extra_headers, p.api_key_query_param.as_deref()),
            None => AuthOptions::default(),
        };
        build_provider_endpoint(protocol, base_url, key, auth)
    }
}

/// Shared endpoint construction for chat + discovery (AuthOptions from settings).
pub fn build_provider_endpoint(
    protocol: ProtocolKind,
    base_url: impl AsRef<str>,
    api_key: impl Into<SecretString>,
    auth: AuthOptions,
) -> Result<ProviderEndpoint, AppError> {
    let mut endpoint = ProviderEndpoint::new(protocol, base_url, api_key)?;
    endpoint.auth = auth;
    Ok(endpoint)
}

pub fn stored_auth_options(
    extra_headers: &[crate::storage::StoredExtraHeader],
    api_key_query_param: Option<&str>,
) -> AuthOptions {
    AuthOptions {
        extra_headers: extra_headers
            .iter()
            .map(|h| (h.name.clone(), h.value.clone()))
            .collect(),
        api_key_query_param: api_key_query_param
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
    }
}

fn map_key_update(input: ApiKeyUpdateInput) -> ApiKeyUpdate {
    match input {
        ApiKeyUpdateInput::Keep => ApiKeyUpdate::Keep,
        ApiKeyUpdateInput::Replace { value } => ApiKeyUpdate::Replace(value),
        ApiKeyUpdateInput::Clear => ApiKeyUpdate::Clear,
    }
}

pub fn parse_protocol(raw: &str) -> Result<StoredProtocol, AppError> {
    match raw {
        "openai_chat_completions" => Ok(StoredProtocol::OpenAiChatCompletions),
        "openai_responses" => Ok(StoredProtocol::OpenAiResponses),
        "anthropic_messages" => Ok(StoredProtocol::AnthropicMessages),
        "gemini_generate_content" => Ok(StoredProtocol::GeminiGenerateContent),
        _ => Err(AppError::validation("不支持的协议")),
    }
}

pub fn parse_protocol_kind(raw: &str) -> Result<ProtocolKind, AppError> {
    parse_protocol(raw).map(protocol_kind)
}

pub fn protocol_kind(protocol: StoredProtocol) -> ProtocolKind {
    match protocol {
        StoredProtocol::OpenAiChatCompletions => ProtocolKind::OpenAiChatCompletions,
        StoredProtocol::OpenAiResponses => ProtocolKind::OpenAiResponses,
        StoredProtocol::AnthropicMessages => ProtocolKind::AnthropicMessages,
        StoredProtocol::GeminiGenerateContent => ProtocolKind::GeminiGenerateContent,
    }
}

pub fn protocol_wire_id(kind: ProtocolKind) -> &'static str {
    match kind {
        ProtocolKind::OpenAiChatCompletions => "openai_chat_completions",
        ProtocolKind::OpenAiResponses => "openai_responses",
        ProtocolKind::AnthropicMessages => "anthropic_messages",
        ProtocolKind::GeminiGenerateContent => "gemini_generate_content",
    }
}

fn capabilities_payload(
    kind: ProtocolKind,
    caps: ProtocolCapabilities,
) -> ProtocolCapabilitiesPayload {
    ProtocolCapabilitiesPayload {
        protocol: protocol_wire_id(kind).to_string(),
        streaming: caps.streaming,
        list_models: caps.list_models,
        reasoning_output: caps.reasoning_output,
        reasoning_control: caps.reasoning_control,
        tools: caps.tools,
        vision_input: caps.vision_input,
        stream_usage: caps.stream_usage,
        custom_headers: caps.custom_headers,
        api_key_query: caps.api_key_query,
    }
}

impl From<ModelInfo> for ModelInfoPayload {
    fn from(model: ModelInfo) -> Self {
        Self {
            id: model.id,
            display_name: model.display_name,
            owned_by: model.owned_by,
        }
    }
}
