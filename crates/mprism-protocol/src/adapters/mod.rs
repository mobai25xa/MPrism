//! Protocol adapters.
//!
//! Each sub-module owns vendor wire formats. Shared semantics live in the crate root.

pub mod anthropic;
pub mod gemini;
pub mod openai;
pub mod openai_responses;

pub use anthropic::AnthropicMessagesAdapter;
pub use gemini::GeminiGenerateContentAdapter;
pub use openai::OpenAiCompatibleAdapter;
pub use openai_responses::OpenAiResponsesAdapter;
