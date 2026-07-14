//! MPrism model protocol SDK.
//!
//! Adapters: OpenAI-compatible Chat Completions, OpenAI Responses, Anthropic Messages,
//! and Gemini generateContent. Desktop enables them via AdapterRegistry (see app_sdk.md).
//! This crate must not depend on Tauri, UI types, or application storage.
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
//!     messages: vec![ChatMessage {
//!         role: ChatRole::User,
//!         content: "Hello".into(),
//!     }],
//!     temperature: Some(0.7),
//!     max_tokens: Some(128),
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
mod error;
mod secret;
mod sse;
mod types;

pub use adapter::{ChatStream, ProtocolAdapter};
pub use adapters::{
    AnthropicMessagesAdapter, GeminiGenerateContentAdapter, OpenAiCompatibleAdapter,
    OpenAiResponsesAdapter,
};
pub use error::{ProtocolError, ProtocolErrorKind};
pub use secret::SecretString;
pub use types::{
    join_api_path, normalize_base_url, ChatMessage, ChatRequest, ChatRole, ModelInfo, ProtocolKind,
    ProviderEndpoint, StreamEvent, TokenUsage,
};

/// Crate version string for diagnostics.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
