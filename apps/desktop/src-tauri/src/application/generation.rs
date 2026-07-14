use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::AppError;

struct GenerationEntry {
    session_id: Uuid,
    cancellation: CancellationToken,
}

#[derive(Default)]
struct GenerationIndexes {
    by_session: HashMap<Uuid, Uuid>,
    by_request: HashMap<Uuid, GenerationEntry>,
}

pub struct GenerationManager {
    indexes: Mutex<GenerationIndexes>,
    accepting: AtomicBool,
    changed: Notify,
}

impl GenerationManager {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            indexes: Mutex::new(GenerationIndexes::default()),
            accepting: AtomicBool::new(true),
            changed: Notify::new(),
        })
    }

    pub fn register(
        self: &Arc<Self>,
        session_id: Uuid,
        request_id: Uuid,
    ) -> Result<(GenerationGuard, CancellationToken), AppError> {
        if !self.accepting.load(Ordering::Acquire) {
            return Err(AppError::conflict("应用正在退出，不能开始新的生成"));
        }
        let mut indexes = self.indexes.lock();
        if indexes.by_session.contains_key(&session_id) {
            return Err(AppError::conflict("generation_already_running"));
        }
        let cancellation = CancellationToken::new();
        indexes.by_session.insert(session_id, request_id);
        indexes.by_request.insert(
            request_id,
            GenerationEntry {
                session_id,
                cancellation: cancellation.clone(),
            },
        );
        drop(indexes);
        Ok((
            GenerationGuard {
                manager: Arc::clone(self),
                session_id,
                request_id,
                active: true,
            },
            cancellation,
        ))
    }

    pub fn cancel(&self, request_id: Uuid) -> bool {
        let indexes = self.indexes.lock();
        let Some(entry) = indexes.by_request.get(&request_id) else {
            return false;
        };
        entry.cancellation.cancel();
        true
    }

    pub fn cancel_all(&self) {
        let indexes = self.indexes.lock();
        for entry in indexes.by_request.values() {
            entry.cancellation.cancel();
        }
    }

    pub fn stop_accepting(&self) {
        self.accepting.store(false, Ordering::Release);
    }

    pub fn running_count(&self) -> usize {
        self.indexes.lock().by_request.len()
    }

    pub async fn shutdown(&self, timeout: std::time::Duration) {
        self.stop_accepting();
        self.cancel_all();
        let wait = async {
            while self.running_count() != 0 {
                self.changed.notified().await;
            }
        };
        let _ = tokio::time::timeout(timeout, wait).await;
    }

    fn unregister(&self, session_id: Uuid, request_id: Uuid) {
        let mut indexes = self.indexes.lock();
        if indexes.by_session.get(&session_id) == Some(&request_id) {
            indexes.by_session.remove(&session_id);
        }
        if indexes
            .by_request
            .get(&request_id)
            .map(|entry| entry.session_id)
            == Some(session_id)
        {
            indexes.by_request.remove(&request_id);
        }
        drop(indexes);
        self.changed.notify_waiters();
    }
}

pub struct GenerationGuard {
    manager: Arc<GenerationManager>,
    session_id: Uuid,
    request_id: Uuid,
    active: bool,
}

impl Drop for GenerationGuard {
    fn drop(&mut self) {
        if self.active {
            self.manager.unregister(self.session_id, self.request_id);
            self.active = false;
        }
    }
}
