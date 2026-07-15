use std::collections::HashMap;
use std::sync::Arc;

use mprism_protocol::{ProtocolAdapter, ProtocolKind};

use crate::application::AppError;

#[derive(Default)]
pub struct AdapterRegistry {
    adapters: HashMap<ProtocolKind, Arc<dyn ProtocolAdapter>>,
}

impl AdapterRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, adapter: Arc<dyn ProtocolAdapter>) {
        self.adapters.insert(adapter.kind(), adapter);
    }

    pub fn get(&self, kind: ProtocolKind) -> Result<Arc<dyn ProtocolAdapter>, AppError> {
        self.adapters
            .get(&kind)
            .cloned()
            .ok_or_else(|| AppError::new("protocol", "当前协议未启用", false))
    }

    pub fn list_kinds(&self) -> Vec<ProtocolKind> {
        let mut kinds: Vec<_> = self.adapters.keys().copied().collect();
        kinds.sort_by_key(|k| format!("{k:?}"));
        kinds
    }
}
