//! Anthropic Messages adapter modules.
//!
//! Wire formats stay private. Public surface is `AnthropicMessagesAdapter` only.

mod client;
mod sse_events;

pub use client::AnthropicMessagesAdapter;
