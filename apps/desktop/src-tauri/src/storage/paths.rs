//! Path helpers and data-root resolution.

use std::path::{Path, PathBuf};

use uuid::Uuid;

use super::error::{StorageError, StorageResult};

/// Schema version written and accepted by this application build.
pub const SCHEMA_VERSION: u32 = 1;

/// Default production data root: `%USERPROFILE%\.mprism`.
pub fn default_data_root() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".mprism")
}

/// Resolve the data root.
///
/// - Production (`not(debug_assertions)`): always [`default_data_root`].
/// - Debug: optional `MPRISM_DATA_ROOT` override for local testing.
pub fn resolve_data_root() -> PathBuf {
    #[cfg(debug_assertions)]
    {
        if let Ok(raw) = std::env::var("MPRISM_DATA_ROOT") {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                return PathBuf::from(trimmed);
            }
        }
    }
    default_data_root()
}

pub fn settings_path(root: &Path) -> PathBuf {
    root.join("settings.json")
}

pub fn device_path(root: &Path) -> PathBuf {
    root.join("device.json")
}

pub fn sessions_dir(root: &Path) -> PathBuf {
    root.join("sessions")
}

pub fn logs_dir(root: &Path) -> PathBuf {
    root.join("logs")
}

pub fn attachments_dir(root: &Path) -> PathBuf {
    root.join("attachments")
}

pub fn attachment_blob_path(root: &Path, attachment_id: Uuid) -> PathBuf {
    attachments_dir(root).join(format!("{attachment_id}.bin"))
}

pub fn attachment_meta_path(root: &Path, attachment_id: Uuid) -> PathBuf {
    attachments_dir(root).join(format!("{attachment_id}.json"))
}

pub fn session_dir(root: &Path, session_id: Uuid) -> PathBuf {
    sessions_dir(root).join(session_id.to_string())
}

pub fn session_meta_path(root: &Path, session_id: Uuid) -> PathBuf {
    session_dir(root, session_id).join("meta.json")
}

pub fn session_messages_path(root: &Path, session_id: Uuid) -> PathBuf {
    session_dir(root, session_id).join("messages.jsonl")
}

/// Parse an IPC UUID string, then re-format for path segments (never trust raw input).
#[allow(dead_code)] // used by phase-4 IPC path validation
pub fn parse_uuid_for_path(raw: &str) -> StorageResult<Uuid> {
    Uuid::parse_str(raw.trim()).map_err(|_| {
        StorageError::UnsafePath(format!("无效的 UUID: {}", redact_path_fragment(raw)))
    })
}

fn redact_path_fragment(raw: &str) -> String {
    let t = raw.trim();
    if t.len() <= 12 {
        return "<invalid>".into();
    }
    format!("{}…", &t[..8])
}

pub fn ensure_parent_dir(path: &Path) -> StorageResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| StorageError::io(parent, e))?;
    }
    Ok(())
}

pub fn check_schema_version(found: u32) -> StorageResult<()> {
    if found > SCHEMA_VERSION {
        return Err(StorageError::SchemaTooNew {
            found,
            supported: SCHEMA_VERSION,
        });
    }
    if found < SCHEMA_VERSION {
        return Err(StorageError::SchemaUnsupported {
            found,
            supported: SCHEMA_VERSION,
        });
    }
    Ok(())
}
