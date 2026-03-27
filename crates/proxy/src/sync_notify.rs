use std::sync::Arc;

#[derive(Clone)]
pub struct SyncCheckNotify {
    inner: Arc<tokio::sync::Notify>,
}

impl SyncCheckNotify {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(tokio::sync::Notify::new()),
        }
    }

    pub fn notify(&self) {
        self.inner.notify_waiters();
    }

    pub async fn notified(&self) {
        self.inner.notified().await;
    }
}
