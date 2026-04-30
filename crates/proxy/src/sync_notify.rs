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
        // Uses `notify_one` over `notify_waiters()`, as `notify_one` stores up
        // to one permit, so a notify fired while the consumer is busy is
        // coalesced and consumed on the next `notified().await`.
        // Multiple notifies-during-busy will collapse into a single wakeup.
        // Needs revisiting if we add a second consumer.
        self.inner.notify_one();
    }

    pub async fn notified(&self) {
        self.inner.notified().await;
    }
}
