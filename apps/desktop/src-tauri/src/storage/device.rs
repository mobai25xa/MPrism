//! device.json load/create.

use std::path::Path;

use super::atomic::{atomic_write_json, read_to_string};
use super::error::{StorageError, StorageResult};
use super::paths::{check_schema_version, device_path};
use super::types::DeviceDocument;

pub fn load_or_create_device(root: &Path) -> StorageResult<DeviceDocument> {
    let path = device_path(root);
    if !path.exists() {
        let doc = DeviceDocument::new();
        atomic_write_json(&path, &doc)?;
        return Ok(doc);
    }
    let raw = read_to_string(&path)?;
    let doc: DeviceDocument =
        serde_json::from_str(&raw).map_err(|e| StorageError::json(&path, e))?;
    check_schema_version(doc.schema_version)?;
    Ok(doc)
}
