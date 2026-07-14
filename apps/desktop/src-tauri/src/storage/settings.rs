//! settings.json CRUD and provider management.

use std::collections::HashSet;
use std::path::Path;

use time::OffsetDateTime;
use uuid::Uuid;

use super::atomic::{atomic_write_json, read_to_string};
use super::error::{StorageError, StorageResult};
use super::paths::{check_schema_version, settings_path, SCHEMA_VERSION};
use super::types::{
    char_len, normalize_stored_base_url, ModelRecord, ProviderRecord, SettingsDocument,
    StoredProtocol, ThemePreference, MAX_PROVIDER_NAME_CHARS,
};

/// Three-state API key update for upserts.
#[derive(Debug, Clone)]
pub enum ApiKeyUpdate {
    Keep,
    Replace(String),
    Clear,
}

/// Input for creating or updating a provider (no raw key leakage in Debug).
#[derive(Clone)]
pub struct ProviderUpsert {
    pub id: Option<Uuid>,
    pub name: String,
    pub protocol: StoredProtocol,
    pub base_url: String,
    pub api_key: ApiKeyUpdate,
    pub models: Vec<ModelRecord>,
}

impl std::fmt::Debug for ProviderUpsert {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderUpsert")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("protocol", &self.protocol)
            .field("base_url", &self.base_url)
            .field(
                "api_key",
                &match &self.api_key {
                    ApiKeyUpdate::Keep => "keep",
                    ApiKeyUpdate::Replace(_) => "replace(***)",
                    ApiKeyUpdate::Clear => "clear",
                },
            )
            .field("models", &self.models)
            .finish()
    }
}

/// Public provider view for IPC (never includes plaintext key).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProviderPublic {
    pub id: Uuid,
    pub name: String,
    pub protocol: StoredProtocol,
    pub base_url: String,
    pub api_key_present: bool,
    pub models: Vec<ModelRecord>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    pub revision: u64,
}

impl From<&ProviderRecord> for ProviderPublic {
    fn from(p: &ProviderRecord) -> Self {
        Self {
            id: p.id,
            name: p.name.clone(),
            protocol: p.protocol,
            base_url: p.base_url.clone(),
            api_key_present: !p.api_key.is_empty(),
            models: p.models.clone(),
            created_at: p.created_at,
            updated_at: p.updated_at,
            revision: p.revision,
        }
    }
}

pub fn load_or_create_settings(root: &Path) -> StorageResult<SettingsDocument> {
    let path = settings_path(root);
    if !path.exists() {
        let doc = SettingsDocument::new();
        atomic_write_json(&path, &doc)?;
        return Ok(doc);
    }
    let raw = read_to_string(&path)?;
    let mut doc: SettingsDocument =
        serde_json::from_str(&raw).map_err(|e| StorageError::json(&path, e))?;
    check_schema_version(doc.schema_version)?;
    doc.repair_defaults();
    Ok(doc)
}

pub fn save_settings(root: &Path, doc: &SettingsDocument) -> StorageResult<()> {
    if doc.schema_version != SCHEMA_VERSION {
        return Err(StorageError::SchemaUnsupported {
            found: doc.schema_version,
            supported: SCHEMA_VERSION,
        });
    }
    atomic_write_json(&settings_path(root), doc)
}

pub fn set_theme(root: &Path, theme: ThemePreference) -> StorageResult<SettingsDocument> {
    let mut doc = load_or_create_settings(root)?;
    doc.theme = theme;
    doc.touch();
    save_settings(root, &doc)?;
    Ok(doc)
}

pub fn upsert_provider(
    root: &Path,
    input: ProviderUpsert,
) -> StorageResult<(SettingsDocument, ProviderPublic)> {
    let mut doc = load_or_create_settings(root)?;
    let name = input.name.trim().to_string();
    let nlen = char_len(&name);
    if nlen == 0 || nlen > MAX_PROVIDER_NAME_CHARS {
        return Err(StorageError::validation(format!(
            "服务商名称长度须为 1–{MAX_PROVIDER_NAME_CHARS} 个字符"
        )));
    }
    let base_url = normalize_stored_base_url(&input.base_url)?;

    let mut model_ids = HashSet::new();
    for m in &input.models {
        m.validate()?;
        if !model_ids.insert(m.id.clone()) {
            return Err(StorageError::validation(format!(
                "同一服务商内 model id 重复: {}",
                m.id
            )));
        }
    }

    let now = OffsetDateTime::now_utc();
    let public = if let Some(id) = input.id {
        let idx = doc
            .providers
            .iter()
            .position(|p| p.id == id)
            .ok_or_else(|| StorageError::not_found(format!("服务商不存在: {id}")))?;
        let existing = &mut doc.providers[idx];
        existing.name = name;
        existing.protocol = input.protocol;
        existing.base_url = base_url;
        match input.api_key {
            ApiKeyUpdate::Keep => {}
            ApiKeyUpdate::Replace(v) => existing.api_key = v,
            ApiKeyUpdate::Clear => existing.api_key.clear(),
        }
        existing.models = input.models;
        existing.updated_at = now;
        existing.revision = existing.revision.saturating_add(1);
        ProviderPublic::from(&*existing)
    } else {
        let api_key = match input.api_key {
            ApiKeyUpdate::Keep | ApiKeyUpdate::Clear => String::new(),
            ApiKeyUpdate::Replace(v) => v,
        };
        let record = ProviderRecord {
            id: Uuid::now_v7(),
            name,
            protocol: input.protocol,
            base_url,
            api_key,
            models: input.models,
            created_at: now,
            updated_at: now,
            revision: 1,
        };
        let public = ProviderPublic::from(&record);
        doc.providers.push(record);
        public
    };

    doc.repair_defaults();
    doc.touch();
    save_settings(root, &doc)?;
    Ok((doc, public))
}

pub fn delete_provider(root: &Path, provider_id: Uuid) -> StorageResult<SettingsDocument> {
    let mut doc = load_or_create_settings(root)?;
    let before = doc.providers.len();
    doc.providers.retain(|p| p.id != provider_id);
    if doc.providers.len() == before {
        return Err(StorageError::not_found(format!(
            "服务商不存在: {provider_id}"
        )));
    }
    doc.repair_defaults();
    doc.touch();
    save_settings(root, &doc)?;
    Ok(doc)
}

pub fn set_defaults(
    root: &Path,
    provider_id: Option<Uuid>,
    model_id: Option<String>,
) -> StorageResult<SettingsDocument> {
    let mut doc = load_or_create_settings(root)?;
    match (provider_id, model_id) {
        (None, None) => {
            doc.default_provider_id = None;
            doc.default_model_id = None;
        }
        (Some(pid), Some(mid)) => {
            let p = doc
                .providers
                .iter()
                .find(|p| p.id == pid)
                .ok_or_else(|| StorageError::not_found(format!("服务商不存在: {pid}")))?;
            if !p.models.iter().any(|m| m.id == mid) {
                return Err(StorageError::validation(
                    "default_model_id 必须属于 default_provider_id",
                ));
            }
            doc.default_provider_id = Some(pid);
            doc.default_model_id = Some(mid);
        }
        _ => {
            return Err(StorageError::validation(
                "default_provider_id 与 default_model_id 必须同时设置或同时清空",
            ));
        }
    }
    doc.touch();
    save_settings(root, &doc)?;
    Ok(doc)
}

pub fn providers_public(doc: &SettingsDocument) -> Vec<ProviderPublic> {
    doc.providers.iter().map(ProviderPublic::from).collect()
}
