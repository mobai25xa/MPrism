//! MPrism model protocol SDK.
//!
//! Adapters: OpenAI-compatible Chat Completions, OpenAI Responses, Anthropic Messages,
//! and Gemini generateContent. Desktop enables them via AdapterRegistry (see app_sdk.md).
//! This crate must not depend on Tauri, UI types, or application storage.
//!
//! # Public model (V2)
//!
//! - [`ChatMessage::text`] builds pure-text messages (V1-compatible path).
//! - [`ReasoningPolicy`] is **request-side** control; [`StreamEvent::ReasoningDelta`] is
//!   **response-side** visible text. They are not the same concept.
//! - [`ToolDefinition`] / tool stream events are **wire-only**; this crate does not execute tools.
//! - Capability gates: [`ChatRequest::check_capabilities`].
//!
//! # Example
//!
//! ```no_run
//! use mprism_protocol::{
//!     ChatMessage, ChatRequest, ChatRole, OpenAiCompatibleAdapter, ProtocolAdapter,
//!     ProtocolKind, ProviderEndpoint, StreamEvent,
//! };
//! use futures_util::StreamExt;
//!
//! # async fn demo() -> Result<(), mprism_protocol::ProtocolError> {
//! let adapter = OpenAiCompatibleAdapter::new()?;
//! let endpoint = ProviderEndpoint::new(
//!     ProtocolKind::OpenAiChatCompletions,
//!     "https://api.example.com/v1",
//!     "sk-example",
//! )?;
//! let models = adapter.list_models(&endpoint).await?;
//! let request = ChatRequest {
//!     model: models[0].id.clone(),
//!     messages: vec![ChatMessage::text(ChatRole::User, "Hello")],
//!     temperature: Some(0.7),
//!     max_tokens: Some(128),
//!     reasoning: None,
//!     tools: None,
//!     tool_choice: None,
//! };
//! let mut stream = adapter.stream_chat(&endpoint, request).await?;
//! while let Some(event) = stream.next().await {
//!     match event? {
//!         StreamEvent::ContentDelta { text } => print!("{text}"),
//!         StreamEvent::Completed { .. } => break,
//!         _ => {}
//!     }
//! }
//! # Ok(())
//! # }
//! ```

mod adapter;
mod adapters;
mod auth;
mod error;
mod finish;
mod secret;
mod sse;
mod types;
mod usage;

pub use adapter::{ChatStream, ProtocolAdapter};
pub use adapters::{
    AnthropicMessagesAdapter, GeminiGenerateContentAdapter, OpenAiCompatibleAdapter,
    OpenAiResponsesAdapter,
};
pub use auth::{apply_api_key_query, merge_extra_headers};
pub use error::{
    kind_from_status, map_http_error, parse_anthropic_error_body, parse_gemini_error_body,
    parse_openai_error_body, parse_provider_error_body, parse_retry_after, redact_secrets,
    upgrade_kind_from_body, ErrorBodyFamily, ProtocolError, ProtocolErrorKind, ERROR_BODY_LIMIT,
};
pub use finish::{
    anthropic_messages as finish_reason_anthropic_messages,
    gemini_generate_content as finish_reason_gemini_generate_content,
    openai_chat_completions as finish_reason_openai_chat_completions,
    openai_responses as finish_reason_openai_responses,
};
pub use secret::SecretString;
pub use types::{
    join_api_path, normalize_base_url, AuthOptions, ChatMessage, ChatRequest, ChatRole,
    ContentPart, FinishReason, ModelInfo, ProtocolCapabilities, ProtocolKind, ProviderEndpoint,
    ReasoningEffort, ReasoningMode, ReasoningPolicy, StreamEvent, TokenUsage, ToolCall, ToolChoice,
    ToolDefinition, MAX_INLINE_IMAGE_BYTES,
};

/// Crate version string for diagnostics.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
