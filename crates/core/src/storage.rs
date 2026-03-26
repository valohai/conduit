use std::collections::HashMap;
use std::future::Future;
use std::num::NonZeroU32;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::Value;
use uuid::Uuid;

use crate::Provider;

#[derive(Clone)]
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

    fn get_transits(
        &self,
        transit_ids: Vec<Uuid>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<TransitRecord>>> + Send + '_>>;
}

pub struct IdentityDeclaration {
    pub transit_id: Uuid,
    pub provider: Provider,
    pub header_id: Option<String>,
    pub body_id: Option<String>,
    pub vh_headers: Option<HashMap<String, String>>,
}

pub struct UsageDeclaration {
    pub transit_id: Uuid,
    pub provider: Provider,
    pub model: Option<String>,
    pub usage: Value,
}

pub struct TransitRecord {
    pub transit_id: Uuid,
    pub stored_at: chrono::DateTime<chrono::Utc>,
    pub provider: Provider,
    pub header_id: Option<String>, // LLM provider's identifier from the response headers i.e. "debugging id"
    pub body_id: Option<String>, // LLM provider's identifier from the response body i.e. "correlation id"
    pub vh_headers: Option<HashMap<String, String>>, // "X-VH-" headers from the request
    pub model: Option<String>,
    pub usage: Option<Value>,
    pub estimated_cost_usd: Option<f64>,
}

impl TransitRecord {
    pub fn estimate_cost(&self) -> Option<f64> {
        if let Some(cost) = self.estimated_cost_usd {
            return Some(cost);
        }
        let model = self.model.as_deref()?;
        let usage = self.usage.as_ref()?;
        crate::cost::estimate_cost(self.provider, model, usage)
    }
}

pub struct TransitQuery {
    pub cursor: Option<chrono::DateTime<chrono::Utc>>,
    pub direction: Direction,
    pub limit: NonZeroU32,
}

pub enum Direction {
    After,  // records after the cursor, or from first existing if no cursor
    Before, // records before the cursor, or from last existing if no cursor
}

pub struct TransitPage {
    pub records: Vec<TransitRecord>,
    pub has_more: bool,
}
