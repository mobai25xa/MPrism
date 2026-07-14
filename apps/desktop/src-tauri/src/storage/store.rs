//! FileStore facade: layout init, settings, sessions, messages, locks.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use uuid::Uuid;

use super::atomic::cleanup_stale_temp_files;
use super::device;
use super::error::{StorageError, StorageResult};
use super::logs::AppLogger;
use super::messages::{self, LoadMessagesResult};
use super::paths::{self, logs_dir, sessions_dir};
use super::sessions::{self, SessionUpdate};
use super::settings::{self, ProviderPublic, ProviderUpsert};
use super::types::{DeviceDocument, MessageRecord, SessionMeta, SettingsDocument, ThemePreference};

/// Local filesystem store rooted at `.mprism`.
pub struct FileStore {
    root: PathBuf,
    device: DeviceDocument,
    session_locks: Mutex<HashMap<Uuid, Arc<Mutex<()>>>>,
    logger: AppLogger,
}

impl FileStore {
    /// Open an existing or new data root (tests inject temp dirs).
    pub fn open(root: impl Into<PathBuf>) -> StorageResult<Self> {
        let root = root.into();
        fs::create_dir_all(&root).map_err(|e| StorageError::io(&root, e))?;
        fs::create_dir_all(sessions_dir(&root))
            .map_err(|e| StorageError::io(sessions_dir(&root), e))?;
        fs::create_dir_all(logs_dir(&root)).map_err(|e| StorageError::io(logs_dir(&root), e))?;
        let _ = cleanup_stale_temp_files(&root);
        let device = device::load_or_create_device(&root)?;
        // Ensure settings exist.
        let _ = settings::load_or_create_settings(&root)?;
        let logger = AppLogger::new(&root);
        logger.info(&format!(
            "FileStore opened root={} device={}",
            root.display(),
            device.device_id
        ));
        Ok(Self {
            root,
            device,
            session_locks: Mutex::new(HashMap::new()),
            logger,
        })
    }

    /// Open the production/debug resolved data root.
    pub fn open_default() -> StorageResult<Self> {
        Self::open(paths::resolve_data_root())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn device(&self) -> &DeviceDocument {
        &self.device
    }

    pub fn device_id(&self) -> Uuid {
        self.device.device_id
    }

    pub fn logger(&self) -> &AppLogger {
        &self.logger
    }

    fn lock_session(&self, session_id: Uuid) -> Arc<Mutex<()>> {
        let mut map = self.session_locks.lock();
        map.entry(session_id)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    // --- settings ---

    pub fn load_settings(&self) -> StorageResult<SettingsDocument> {
        settings::load_or_create_settings(&self.root)
    }

    pub fn save_settings(&self, doc: &SettingsDocument) -> StorageResult<()> {
        settings::save_settings(&self.root, doc)
    }

    pub fn set_theme(&self, theme: ThemePreference) -> StorageResult<SettingsDocument> {
        settings::set_theme(&self.root, theme)
    }

    pub fn upsert_provider(
        &self,
        input: ProviderUpsert,
    ) -> StorageResult<(SettingsDocument, ProviderPublic)> {
        settings::upsert_provider(&self.root, input)
    }

    pub fn delete_provider(&self, provider_id: Uuid) -> StorageResult<SettingsDocument> {
        settings::delete_provider(&self.root, provider_id)
    }

    pub fn set_defaults(
        &self,
        provider_id: Option<Uuid>,
        model_id: Option<String>,
    ) -> StorageResult<SettingsDocument> {
        settings::set_defaults(&self.root, provider_id, model_id)
    }

    pub fn providers_public(&self) -> StorageResult<Vec<ProviderPublic>> {
        let doc = self.load_settings()?;
        Ok(settings::providers_public(&doc))
    }

    // --- sessions ---

    pub fn create_session(&self, title: Option<String>) -> StorageResult<SessionMeta> {
        sessions::create_session(&self.root, self.device_id(), title)
    }

    pub fn list_sessions(&self) -> StorageResult<Vec<SessionMeta>> {
        sessions::list_sessions(&self.root)
    }

    pub fn load_session_meta(&self, session_id: Uuid) -> StorageResult<SessionMeta> {
        sessions::load_meta(&self.root, session_id)
    }

    pub fn update_session(
        &self,
        session_id: Uuid,
        update: SessionUpdate,
    ) -> StorageResult<SessionMeta> {
        let lock = self.lock_session(session_id);
        let _g = lock.lock();
        sessions::update_session(&self.root, session_id, update)
    }

    pub fn delete_session(&self, session_id: Uuid) -> StorageResult<SessionMeta> {
        let lock = self.lock_session(session_id);
        let _g = lock.lock();
        sessions::soft_delete_session(&self.root, session_id)
    }

    // --- messages ---

    pub fn load_messages(&self, session_id: Uuid) -> StorageResult<LoadMessagesResult> {
        let lock = self.lock_session(session_id);
        let _g = lock.lock();
        let result = messages::load_messages(&self.root, session_id)?;
        if result.partially_corrupt {
            self.logger.warn(&format!(
                "session {session_id} messages partially corrupt; warnings={}",
                result.warnings.len()
            ));
        }
        Ok(result)
    }

    /// Append a message under the session lock and bump session meta.
    pub fn append_message_and_touch(
        &self,
        mut message: MessageRecord,
        auto_title_from_user: bool,
    ) -> StorageResult<MessageRecord> {
        let session_id = message.session_id;
        let lock = self.lock_session(session_id);
        let _g = lock.lock();

        // Assign sequence if zero (caller may pre-set).
        if message.sequence == 0 {
            message.sequence = messages::next_sequence(&self.root, session_id)?;
        }

        messages::append_message(&self.root, &message)?;

        let mut meta = sessions::load_meta(&self.root, session_id)?;
        if auto_title_from_user && message.role == super::types::MessageRole::User {
            meta.maybe_auto_title(&message.content);
        }
        meta.touch();
        // If meta save fails, message is still valid (spec 8.2).
        if let Err(e) = sessions::save_meta(&self.root, &meta) {
            self.logger.warn(&format!(
                "session {session_id} meta update failed after message append: {e}"
            ));
        }
        Ok(message)
    }

    pub fn next_message_sequence(&self, session_id: Uuid) -> StorageResult<u64> {
        let lock = self.lock_session(session_id);
        let _g = lock.lock();
        messages::next_sequence(&self.root, session_id)
    }
}
