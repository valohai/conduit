use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::Value;
use uuid::Uuid;

pub struct Storages {
    pub usage: Arc<dyn UsageStorage>,
}

pub trait UsageStorage: Send + Sync {
    fn store_usages(
        &self,
        records: Vec<UsageRecord>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>>;
}

pub struct UsageRecord {
    pub transit_id: Uuid,
    pub usage: Value,
}
