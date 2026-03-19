use std::future::Future;
use std::num::NonZeroU32;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::Value;
use uuid::Uuid;

use crate::Provider;

pub struct Storages {
    pub transit: Arc<dyn TransitStorage>,
}

pub trait TransitStorage: Send + Sync {
    fn store_identities(
        &self,
        declarations: Vec<IdentityDeclaration>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>>;

    fn store_usages(
        &self,
        declarations: Vec<UsageDeclaration>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>>;

    fn list_transits(
        &self,
        query: TransitQuery,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<TransitPage>> + Send + '_>>;
}

pub struct IdentityDeclaration {
    pub transit_id: Uuid,
    pub provider: Provider,
    pub header_id: Option<String>,
    pub body_id: Option<String>,
}

pub struct UsageDeclaration {
    pub transit_id: Uuid,
    pub model: Option<String>,
    pub usage: Value,
}

pub struct TransitRecord {
    pub transit_id: Uuid,
    pub stored_at: chrono::DateTime<chrono::Utc>,
    pub provider: Provider,
    pub header_id: Option<String>, // LLM provider's identifier from the response header i.e. "debugging id"
    pub body_id: Option<String>, // LLM provider's identifier from the response body i.e. "correlation id"
    pub model: Option<String>,
    pub usage: Option<Value>,
}

pub struct TransitQuery {
    pub cursor: Option<chrono::DateTime<chrono::Utc>>,
    pub direction: Direction,
    pub limit: NonZeroU32,
}

pub enum Direction {
    Newer,
    Older,
}

pub struct TransitPage {
    pub records: Vec<TransitRecord>,
    pub has_more: bool,
}
