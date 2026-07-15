//! Local image attachment blobs under `.mprism/attachments/`.
//!
//! Messages only store `{ attachment_id, media_type }` refs — never inline base64 JSONL.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use super::atomic::{atomic_write_json, read_to_string};
use super::error::{StorageError, StorageResult};
use super::paths::{
    attachment_blob_path, attachment_meta_path, attachments_dir, ensure_parent_dir, SCHEMA_VERSION,
};

/// Align with SDK `MAX_INLINE_IMAGE_BYTES` (4 MiB).
pub const MAX_ATTACHMENT_BYTES: u64 = mprism_protocol::MAX_INLINE_IMAGE_BYTES as u64;

const ALLOWED_MEDIA_TYPES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/jpg",
    "image/webp",
    "image/gif",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentMeta {
    pub schema_version: u32,
    pub id: Uuid,
    pub media_type: String,
    pub byte_len: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_name: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize)]
pub struct AttachmentPublic {
    pub id: Uuid,
    pub media_type: String,
    pub byte_len: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_name: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

impl From<&AttachmentMeta> for AttachmentPublic {
    fn from(meta: &AttachmentMeta) -> Self {
        Self {
            id: meta.id,
            media_type: meta.media_type.clone(),
            byte_len: meta.byte_len,
            original_name: meta.original_name.clone(),
            created_at: meta.created_at,
        }
    }
}

pub fn normalize_media_type(raw: &str) -> StorageResult<String> {
    let mt = raw.trim().to_ascii_lowercase();
    let mt = if mt == "image/jpg" {
        "image/jpeg".to_string()
    } else {
        mt
    };
    if !ALLOWED_MEDIA_TYPES
        .iter()
        .any(|a| *a == mt || (*a == "image/jpg" && mt == "image/jpeg"))
        && !matches!(
            mt.as_str(),
            "image/png" | "image/jpeg" | "image/webp" | "image/gif"
        )
    {
        return Err(StorageError::validation(format!(
            "不支持的图片类型: {raw}（允许 png/jpeg/webp/gif）"
        )));
    }
    Ok(mt)
}

/// Persist raw image bytes; returns public metadata (no path/base64).
pub fn import_bytes(
    root: &Path,
    bytes: &[u8],
    media_type: &str,
    original_name: Option<String>,
) -> StorageResult<AttachmentPublic> {
    let media_type = normalize_media_type(media_type)?;
    let byte_len = bytes.len() as u64;
    if byte_len == 0 {
        return Err(StorageError::validation("图片内容不能为空"));
    }
    if byte_len > MAX_ATTACHMENT_BYTES {
        return Err(StorageError::validation(format!(
            "图片超过大小上限 {} 字节",
            MAX_ATTACHMENT_BYTES
        )));
    }

    let id = Uuid::now_v7();
    let blob = attachment_blob_path(root, id);
    let meta_path = attachment_meta_path(root, id);
    ensure_parent_dir(&blob)?;
    fs::create_dir_all(attachments_dir(root))
        .map_err(|e| StorageError::io(attachments_dir(root), e))?;

    fs::write(&blob, bytes).map_err(|e| StorageError::io(&blob, e))?;
    let meta = AttachmentMeta {
        schema_version: SCHEMA_VERSION,
        id,
        media_type,
        byte_len,
        original_name: original_name
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(|s| s.chars().take(200).collect()),
        created_at: OffsetDateTime::now_utc(),
    };
    if let Err(e) = atomic_write_json(&meta_path, &meta) {
        let _ = fs::remove_file(&blob);
        return Err(e);
    }
    Ok(AttachmentPublic::from(&meta))
}

pub fn load_meta(root: &Path, attachment_id: Uuid) -> StorageResult<AttachmentMeta> {
    let path = attachment_meta_path(root, attachment_id);
    if !path.exists() {
        return Err(StorageError::not_found(format!(
            "附件不存在: {attachment_id}"
        )));
    }
    let raw = read_to_string(&path)?;
    let meta: AttachmentMeta =
        serde_json::from_str(&raw).map_err(|e| StorageError::json(&path, e))?;
    Ok(meta)
}

/// Read blob for request assembly. Caller must not log bytes.
pub fn load_bytes(root: &Path, attachment_id: Uuid) -> StorageResult<(AttachmentMeta, Vec<u8>)> {
    let meta = load_meta(root, attachment_id)?;
    let path = attachment_blob_path(root, attachment_id);
    let bytes = fs::read(&path).map_err(|e| StorageError::io(&path, e))?;
    if bytes.len() as u64 != meta.byte_len {
        return Err(StorageError::validation(format!(
            "附件损坏: {attachment_id}"
        )));
    }
    if bytes.len() as u64 > MAX_ATTACHMENT_BYTES {
        return Err(StorageError::validation("附件超过发送大小上限"));
    }
    Ok((meta, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn import_and_load_roundtrip() {
        let dir = tempdir().unwrap();
        let png = [0x89, b'P', b'N', b'G', 0, 1, 2, 3];
        let pub_att = import_bytes(dir.path(), &png, "image/png", Some("a.png".into())).unwrap();
        assert_eq!(pub_att.media_type, "image/png");
        assert_eq!(pub_att.byte_len, png.len() as u64);
        let (meta, bytes) = load_bytes(dir.path(), pub_att.id).unwrap();
        assert_eq!(meta.id, pub_att.id);
        assert_eq!(bytes, png);
    }

    #[test]
    fn rejects_oversize_without_embedding_payload_in_error() {
        let dir = tempdir().unwrap();
        let huge = vec![0u8; (MAX_ATTACHMENT_BYTES as usize) + 1];
        let err = import_bytes(dir.path(), &huge, "image/png", None).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("上限"));
        assert!(!msg.contains(&"AAAA".repeat(20)));
    }
}
