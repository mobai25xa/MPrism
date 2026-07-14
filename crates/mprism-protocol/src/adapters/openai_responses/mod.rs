//! OpenAI Responses API adapter modules.
//!
//! Wire formats stay private. Public surface is `OpenAiResponsesAdapter` only.

mod client;
mod events;

pub use client::OpenAiResponsesAdapter;
