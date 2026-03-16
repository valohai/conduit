use std::future::Future;
use std::num::NonZeroU32;
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
        declarations: Vec<UsageDeclaration>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>>;

    fn list_usages(
        &self,
        query: UsageQuery,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<UsagePage>> + Send + '_>>;
}

pub struct UsageDeclaration {
    pub transit_id: Uuid,
    pub model: Option<String>,
    pub usage: Value,
}

pub struct UsageRecord {
    pub transit_id: Uuid,
    pub stored_at: chrono::DateTime<chrono::Utc>,
    pub model: Option<String>,
    pub usage: Value,
}

pub struct UsageQuery {
    pub cursor: Option<chrono::DateTime<chrono::Utc>>,
    pub direction: Direction,
    pub limit: NonZeroU32,
}

pub enum Direction {
    Newer,
    Older,
}

pub struct UsagePage {
    pub records: Vec<UsageRecord>,
    pub has_more: bool,
}
