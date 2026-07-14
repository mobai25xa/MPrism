//! Protocol adapter trait.

use crate::error::ProtocolError;
use crate::types::{ChatRequest, ModelInfo, ProtocolKind, ProviderEndpoint, StreamEvent};
use async_trait::async_trait;
use futures_core::Stream;
use std::pin::Pin;

/// Streaming chat event source.
pub type ChatStream = Pin<Box<dyn Stream<Item = Result<StreamEvent, ProtocolError>> + Send>>;

/// Provider protocol adapter.
#[async_trait]
pub trait ProtocolAdapter: Send + Sync {
    fn kind(&self) -> ProtocolKind;

    async fn list_models(
        &self,
        endpoint: &ProviderEndpoint,
    ) -> Result<Vec<ModelInfo>, ProtocolError>;

    /// Start a streaming chat request.
    ///
    /// A successful return only means the HTTP response body is ready to read.
    /// Stream items may still yield protocol, timeout, or decode errors.
    async fn stream_chat(
        &self,
        endpoint: &ProviderEndpoint,
        request: ChatRequest,
    ) -> Result<ChatStream, ProtocolError>;
}
