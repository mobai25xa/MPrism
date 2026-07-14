//! Session meta.json CRUD and scanning.

use std::fs;
use std::path::Path;

use time::OffsetDateTime;
use uuid::Uuid;

use super::atomic::{atomic_write_json, read_to_string};
use super::error::{StorageError, StorageResult};
use super::paths::{
    check_schema_version, ensure_parent_dir, session_dir, session_meta_path, sessions_dir,
    SCHEMA_VERSION,
};
use super::types::{SessionMeta, TitleSource, DEFAULT_SESSION_TITLE};

pub fn create_session(
    root: &Path,
    device_id: Uuid,
    title: Option<String>,
) -> StorageResult<SessionMeta> {
    let mut meta = SessionMeta::new(device_id);
    if let Some(t) = title {
        let t = t.trim().to_string();
        if !t.is_empty() {
            meta.title = t;
            meta.title_source = TitleSource::User;
        }
    }
    meta.validate()?;
    let dir = session_dir(root, meta.id);
    fs::create_dir_all(&dir).map_err(|e| StorageError::io(&dir, e))?;
    // Touch empty messages file for consistency.
    let messages = super::paths::session_messages_path(root, meta.id);
    if !messages.exists() {
        fs::write(&messages, b"").map_err(|e| StorageError::io(&messages, e))?;
    }
    save_meta(root, &meta)?;
    Ok(meta)
}

pub fn save_meta(root: &Path, meta: &SessionMeta) -> StorageResult<()> {
    if meta.schema_version != SCHEMA_VERSION {
        return Err(StorageError::SchemaUnsupported {
            found: meta.schema_version,
            supported: SCHEMA_VERSION,
        });
    }
    meta.validate()?;
    let path = session_meta_path(root, meta.id);
    ensure_parent_dir(&path)?;
    atomic_write_json(&path, meta)
}

pub fn load_meta(root: &Path, session_id: Uuid) -> StorageResult<SessionMeta> {
    let path = session_meta_path(root, session_id);
    if !path.exists() {
        return Err(StorageError::not_found(format!("会话不存在: {session_id}")));
    }
    let raw = read_to_string(&path)?;
    let meta: SessionMeta = serde_json::from_str(&raw).map_err(|e| StorageError::json(&path, e))?;
    check_schema_version(meta.schema_version)?;
    if meta.id != session_id {
        return Err(StorageError::Conflict(format!(
            "会话 id 与目录不一致: meta={} dir={session_id}",
            meta.id
        )));
    }
    Ok(meta)
}

#[derive(Debug, Clone, Default)]
pub struct SessionUpdate {
    pub title: Option<String>,
    pub system_prompt: Option<String>,
    pub last_provider_id: Option<Option<Uuid>>,
    pub last_model_id: Option<Option<String>>,
}

pub fn update_session(
    root: &Path,
    session_id: Uuid,
    update: SessionUpdate,
) -> StorageResult<SessionMeta> {
    let mut meta = load_meta(root, session_id)?;
    if meta.deleted_at.is_some() {
        return Err(StorageError::not_found(format!("会话已删除: {session_id}")));
    }
    if let Some(title) = update.title {
        let title = title.trim().to_string();
        meta.title = if title.is_empty() {
            DEFAULT_SESSION_TITLE.to_string()
        } else {
            title
        };
        meta.title_source = TitleSource::User;
    }
    if let Some(sp) = update.system_prompt {
        meta.system_prompt = sp;
    }
    if let Some(pid) = update.last_provider_id {
        meta.last_provider_id = pid;
    }
    if let Some(mid) = update.last_model_id {
        meta.last_model_id = mid;
    }
    meta.touch();
    save_meta(root, &meta)?;
    Ok(meta)
}

pub fn soft_delete_session(root: &Path, session_id: Uuid) -> StorageResult<SessionMeta> {
    let mut meta = load_meta(root, session_id)?;
    if meta.deleted_at.is_some() {
        return Ok(meta);
    }
    meta.deleted_at = Some(OffsetDateTime::now_utc());
    meta.touch();
    save_meta(root, &meta)?;
    Ok(meta)
}

/// List non-deleted sessions ordered by updated_at desc, id desc.
pub fn list_sessions(root: &Path) -> StorageResult<Vec<SessionMeta>> {
    let dir = sessions_dir(root);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let rd = fs::read_dir(&dir).map_err(|e| StorageError::io(&dir, e))?;
    for entry in rd.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Ok(id) = Uuid::parse_str(name) else {
            continue;
        };
        match load_meta(root, id) {
            Ok(meta) if meta.deleted_at.is_none() => out.push(meta),
            Ok(_) => {}
            Err(StorageError::SchemaTooNew { .. }) => {
                // Skip sessions with future schema; do not block listing.
            }
            Err(StorageError::Json { .. }) | Err(StorageError::Io { .. }) => {
                // Corrupted session meta must not block others.
            }
            Err(_) => {}
        }
    }
    out.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| b.id.cmp(&a.id))
    });
    Ok(out)
}
