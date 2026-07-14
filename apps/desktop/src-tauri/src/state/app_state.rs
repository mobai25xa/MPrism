use std::path::PathBuf;
use std::sync::Arc;

use mprism_protocol::{
    AnthropicMessagesAdapter, GeminiGenerateContentAdapter, OpenAiCompatibleAdapter,
    OpenAiResponsesAdapter,
};
use parking_lot::RwLock;

use crate::application::{
    AppError, ChatService, GenerationManager, ProviderService, SessionService,
};
use crate::storage::{FileStore, SettingsDocument};

use super::AdapterRegistry;

pub struct AppState {
    pub data_root: PathBuf,
    pub store: Arc<FileStore>,
    pub settings: Arc<RwLock<SettingsDocument>>,
    pub adapters: Arc<AdapterRegistry>,
    pub generations: Arc<GenerationManager>,
    pub providers: Arc<ProviderService>,
    pub sessions: Arc<SessionService>,
    pub chat: Arc<ChatService>,
}

impl AppState {
    pub fn initialize(root: Option<PathBuf>) -> Result<Self, AppError> {
        let store = Arc::new(match root {
            Some(root) => FileStore::open(root)?,
            None => FileStore::open_default()?,
        });
        let settings = Arc::new(RwLock::new(store.load_settings()?));
        let mut registry = AdapterRegistry::new();
        registry.register(Arc::new(OpenAiCompatibleAdapter::new()?));
        registry.register(Arc::new(OpenAiResponsesAdapter::new()?));
        registry.register(Arc::new(AnthropicMessagesAdapter::new()?));
        registry.register(Arc::new(GeminiGenerateContentAdapter::new()?));
        let adapters = Arc::new(registry);
        let generations = GenerationManager::new();
        let providers = Arc::new(ProviderService::new(
            Arc::clone(&store),
            Arc::clone(&settings),
            Arc::clone(&adapters),
        ));
        let sessions = Arc::new(SessionService::new(Arc::clone(&store)));
        let chat = Arc::new(ChatService::new(
            Arc::clone(&store),
            Arc::clone(&settings),
            Arc::clone(&adapters),
            Arc::clone(&sessions),
            Arc::clone(&generations),
        ));
        Ok(Self {
            data_root: store.root().to_path_buf(),
            store,
            settings,
            adapters,
            generations,
            providers,
            sessions,
            chat,
        })
    }

    pub fn from_parts(
        store: Arc<FileStore>,
        adapters: Arc<AdapterRegistry>,
        generations: Arc<GenerationManager>,
    ) -> Result<Self, AppError> {
        let settings = Arc::new(RwLock::new(store.load_settings()?));
        let providers = Arc::new(ProviderService::new(
            Arc::clone(&store),
            Arc::clone(&settings),
            Arc::clone(&adapters),
        ));
        let sessions = Arc::new(SessionService::new(Arc::clone(&store)));
        let chat = Arc::new(ChatService::new(
            Arc::clone(&store),
            Arc::clone(&settings),
            Arc::clone(&adapters),
            Arc::clone(&sessions),
            Arc::clone(&generations),
        ));
        Ok(Self {
            data_root: store.root().to_path_buf(),
            store,
            settings,
            adapters,
            generations,
            providers,
            sessions,
            chat,
        })
    }

    pub async fn shutdown(&self) {
        self.generations
            .shutdown(std::time::Duration::from_secs(2))
            .await;
    }
}
