//! Local JSON/JSONL file storage for MPrism.
//!
//! Production data root is `%USERPROFILE%\.mprism`. Tests inject a temporary root.

mod atomic;
mod attachments;
mod device;
mod error;
mod logs;
mod messages;
mod paths;
mod sessions;
mod settings;
mod store;
mod types;

pub use attachments::{
    import_bytes as import_attachment_bytes, load_bytes as load_attachment_bytes,
    load_meta as load_attachment_meta, AttachmentMeta, AttachmentPublic, MAX_ATTACHMENT_BYTES,
};
pub use error::{StorageError, StorageResult};
pub use logs::{redact_log_message, AppLogger};
pub use messages::{LoadMessagesResult, MessageCorruption, MessageLoadWarning};
pub use paths::{default_data_root, parse_uuid_for_path, resolve_data_root};
pub use sessions::SessionUpdate;
pub use settings::{ApiKeyUpdate, ProviderPublic, ProviderUpsert};
pub use store::FileStore;
pub use types::*;
