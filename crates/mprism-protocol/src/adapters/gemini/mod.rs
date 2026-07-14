//! Gemini generateContent adapter modules.
//!
//! Wire formats stay private. Public surface is `GeminiGenerateContentAdapter` only.

mod client;
mod stream_decode;

pub use client::GeminiGenerateContentAdapter;
