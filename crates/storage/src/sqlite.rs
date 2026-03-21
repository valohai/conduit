use std::future::Future;
use std::pin::Pin;

use conduit_core::{
    Direction, IdentityDeclaration, Provider, TransitPage, TransitQuery, TransitRecord,
    TransitStorage, UsageDeclaration,
};
use sqlx::Row;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use uuid::Uuid;

pub struct SqliteTransitStorage {
    pool: sqlx::SqlitePool,
}

impl SqliteTransitStorage {
    pub async fn new(database_url: &str) -> anyhow::Result<Self> {
        let options = database_url
            .parse::<SqliteConnectOptions>()?
            .journal_mode(SqliteJournalMode::Wal)
            .create_if_missing(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(10)
            .connect_with(options)
            .await?;

        // TODO: add some form of migration system?

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS identity (
                transit_id TEXT PRIMARY KEY NOT NULL,
                stored_at TEXT NOT NULL,
                provider TEXT NOT NULL,
                header_id TEXT,
                body_id TEXT
            )",
        )
        .execute(&pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_identity_stored_at
             ON identity (stored_at)",
        )
        .execute(&pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS usage (
                transit_id TEXT PRIMARY KEY NOT NULL,
                model TEXT,
                usage_json TEXT NOT NULL,
                estimated_cost_usd REAL
            )",
        )
        .execute(&pool)
        .await?;

        tracing::debug!(%database_url, "storage initialized");

        Ok(Self { pool })
    }
}

impl TransitStorage for SqliteTransitStorage {
    fn store_identities(
        &self,
        declarations: Vec<IdentityDeclaration>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
        Box::pin(async move {
            let mut tx = self.pool.begin().await?;
            for declaration in &declarations {
                let now = chrono::Utc::now();
                let stored_at = format_timestamp(&now);
                let transit_id = declaration.transit_id.to_string();
                let provider = declaration.provider.to_string();
                sqlx::query(
                    "INSERT INTO identity (transit_id, stored_at, provider, header_id, body_id) VALUES (?, ?, ?, ?, ?)",
                )
                .bind(&transit_id)
                .bind(&stored_at)
                .bind(&provider)
                .bind(&declaration.header_id)
                .bind(&declaration.body_id)
                .execute(&mut *tx)
                .await?;
            }
            tx.commit().await?;
            tracing::debug!(count = declarations.len(), "stored identity records");
            Ok(())
        })
    }

    fn store_usages(
        &self,
        declarations: Vec<UsageDeclaration>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
        Box::pin(async move {
            let mut tx = self.pool.begin().await?;
            for declaration in &declarations {
                let transit_id = declaration.transit_id.to_string();
                let usage_json = declaration.usage.to_string();
                let estimated_cost_usd = declaration.model.as_deref().and_then(|m| {
                    conduit_core::cost::estimate_cost(declaration.provider, m, &declaration.usage)
                });
                sqlx::query("INSERT INTO usage (transit_id, model, usage_json, estimated_cost_usd) VALUES (?, ?, ?, ?)")
                    .bind(&transit_id)
                    .bind(&declaration.model)
                    .bind(&usage_json)
                    .bind(estimated_cost_usd)
                    .execute(&mut *tx)
                    .await?;
            }
            tx.commit().await?;
            tracing::debug!(count = declarations.len(), "stored usage records");
            Ok(())
        })
    }

    fn list_transits(
        &self,
        query: TransitQuery,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<TransitPage>> + Send + '_>> {
        Box::pin(async move {
            let limit = query.limit.get();
            let fetch_limit = limit + 1;
            let limit = limit as usize;

            let rows = match (&query.cursor, &query.direction) {
                (Some(cursor), Direction::After) => {
                    let stored_at = format_timestamp(cursor);
                    sqlx::query(
                        "SELECT i.transit_id, i.stored_at, i.provider, i.header_id, i.body_id,
                                u.model, u.usage_json, u.estimated_cost_usd
                         FROM identity i
                         LEFT JOIN usage u ON i.transit_id = u.transit_id
                         WHERE i.stored_at > ?
                         ORDER BY i.stored_at ASC
                         LIMIT ?",
                    )
                    .bind(&stored_at)
                    .bind(fetch_limit)
                    .fetch_all(&self.pool)
                    .await?
                }
                (Some(cursor), Direction::Before) => {
                    let stored_at = format_timestamp(cursor);
                    sqlx::query(
                        "SELECT i.transit_id, i.stored_at, i.provider, i.header_id, i.body_id,
                                u.model, u.usage_json, u.estimated_cost_usd
                         FROM identity i
                         LEFT JOIN usage u ON i.transit_id = u.transit_id
                         WHERE i.stored_at < ?
                         ORDER BY i.stored_at DESC
                         LIMIT ?",
                    )
                    .bind(&stored_at)
                    .bind(fetch_limit)
                    .fetch_all(&self.pool)
                    .await?
                }
                (None, Direction::After) => {
                    sqlx::query(
                        "SELECT i.transit_id, i.stored_at, i.provider, i.header_id, i.body_id,
                                u.model, u.usage_json, u.estimated_cost_usd
                         FROM identity i
                         LEFT JOIN usage u ON i.transit_id = u.transit_id
                         ORDER BY i.stored_at ASC
                         LIMIT ?",
                    )
                    .bind(fetch_limit)
                    .fetch_all(&self.pool)
                    .await?
                }
                (None, Direction::Before) => {
                    sqlx::query(
                        "SELECT i.transit_id, i.stored_at, i.provider, i.header_id, i.body_id,
                                u.model, u.usage_json, u.estimated_cost_usd
                         FROM identity i
                         LEFT JOIN usage u ON i.transit_id = u.transit_id
                         ORDER BY i.stored_at DESC
                         LIMIT ?",
                    )
                    .bind(fetch_limit)
                    .fetch_all(&self.pool)
                    .await?
                }
            };

            let has_more = rows.len() > limit;
            let mut records: Vec<TransitRecord> = rows
                .iter()
                .take(limit)
                .filter_map(row_to_transit_record)
                .collect();

            if matches!(query.direction, Direction::After) {
                // result order is still always "stored_at DESC",
                // purely for the newest-at-top display in TUI
                records.reverse();
            }

            Ok(TransitPage { records, has_more })
        })
    }

    fn get_transits(
        &self,
        transit_ids: Vec<Uuid>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<TransitRecord>>> + Send + '_>> {
        Box::pin(async move {
            if transit_ids.is_empty() {
                return Ok(Vec::new());
            }
            let placeholders = transit_ids
                .iter()
                .map(|_| "?")
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "SELECT i.transit_id, i.stored_at, i.provider, i.header_id, i.body_id,
                        u.model, u.usage_json, u.estimated_cost_usd
                 FROM identity i
                 LEFT JOIN usage u ON i.transit_id = u.transit_id
                 WHERE i.transit_id IN ({})
                 ORDER BY i.stored_at ASC",
                placeholders
            );
            let mut query = sqlx::query(&sql);
            for id in &transit_ids {
                query = query.bind(id.to_string());
            }
            let rows = query.fetch_all(&self.pool).await?;
            Ok(rows.iter().filter_map(row_to_transit_record).collect())
        })
    }
}

fn row_to_transit_record(row: &sqlx::sqlite::SqliteRow) -> Option<TransitRecord> {
    let transit_id: String = row.get("transit_id");
    let stored_at: String = row.get("stored_at");
    let provider_str: String = row.get("provider");
    let header_id: Option<String> = row.get("header_id");
    let body_id: Option<String> = row.get("body_id");
    let model: Option<String> = row.get("model");
    let usage_json: Option<String> = row.get("usage_json");
    let estimated_cost_usd: Option<f64> = row.get("estimated_cost_usd");
    let Ok(provider) = provider_str.parse::<Provider>();

    // to keep the proxy process going, be loud about errors but don't panic

    let Ok(transit_id) = transit_id.parse::<Uuid>() else {
        tracing::warn!(transit_id, "invalid UUID in transit record");
        return None;
    };
    let Ok(stored_at) = stored_at.parse::<chrono::DateTime<chrono::Utc>>() else {
        tracing::warn!(%transit_id, stored_at, "invalid timestamp in transit record");
        return None;
    };
    let usage = match usage_json {
        Some(j) => match serde_json::from_str(&j) {
            Ok(v) => Some(v),
            Err(e) => {
                tracing::warn!(%transit_id, error = %e, "invalid usage JSON in transit record");
                return None;
            }
        },
        None => None,
    };

    Some(TransitRecord {
        transit_id,
        stored_at,
        provider,
        header_id,
        body_id,
        model,
        usage,
        estimated_cost_usd,
    })
}

fn format_timestamp(dt: &chrono::DateTime<chrono::Utc>) -> String {
    // with microseconds e.g. 2026-03-16T06:21:03.616874Z
    dt.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;

    use super::*;
    use serde_json::json;
    use uuid::Uuid;

    async fn test_storage() -> SqliteTransitStorage {
        SqliteTransitStorage::new("sqlite::memory:").await.unwrap()
    }

    async fn seed_transit_records(
        storage: &SqliteTransitStorage,
        count: u32,
    ) -> Vec<TransitRecord> {
        let identity_declarations: Vec<_> = (0..count)
            .map(|_| IdentityDeclaration {
                transit_id: Uuid::now_v7(),
                provider: Provider::OpenAI,
                header_id: Some("req-123".into()),
                body_id: Some("chatcmpl-test".into()),
            })
            .collect();
        let transit_ids: Vec<_> = identity_declarations.iter().map(|d| d.transit_id).collect();
        storage
            .store_identities(identity_declarations)
            .await
            .unwrap();

        let usage_declarations: Vec<_> = transit_ids
            .iter()
            .map(|&transit_id| UsageDeclaration {
                transit_id,
                provider: Provider::OpenAI,
                model: Some("test-model".into()),
                usage: json!({"input_tokens": 10, "output_tokens": 20}),
            })
            .collect();
        storage.store_usages(usage_declarations).await.unwrap();

        let page = storage
            .list_transits(TransitQuery {
                cursor: None,
                direction: Direction::After,
                limit: NonZeroU32::new(count).unwrap(),
            })
            .await
            .unwrap();

        page.records
    }

    #[tokio::test]
    async fn list_latest_records() {
        let storage = test_storage().await;
        let all = seed_transit_records(&storage, 5).await;

        let page = storage
            .list_transits(TransitQuery {
                cursor: None,
                direction: Direction::Before,
                limit: NonZeroU32::new(3).unwrap(),
            })
            .await
            .unwrap();

        assert_eq!(page.records.len(), 3);
        assert!(page.has_more);
        assert_eq!(page.records[0].transit_id, all[0].transit_id);
        assert_eq!(page.records[1].transit_id, all[1].transit_id);
        assert_eq!(page.records[2].transit_id, all[2].transit_id);
    }

    #[tokio::test]
    async fn list_oldest_records() {
        let storage = test_storage().await;
        let all = seed_transit_records(&storage, 5).await;

        let page = storage
            .list_transits(TransitQuery {
                cursor: None,
                direction: Direction::After,
                limit: NonZeroU32::new(3).unwrap(),
            })
            .await
            .unwrap();

        assert_eq!(page.records.len(), 3);
        assert!(page.has_more);
        assert_eq!(page.records[0].transit_id, all[2].transit_id);
        assert_eq!(page.records[1].transit_id, all[3].transit_id);
        assert_eq!(page.records[2].transit_id, all[4].transit_id);
    }

    #[tokio::test]
    async fn list_after_cursor() {
        let storage = test_storage().await;
        let all = seed_transit_records(&storage, 5).await;

        let page = storage
            .list_transits(TransitQuery {
                cursor: Some(all[3].stored_at),
                direction: Direction::After,
                limit: NonZeroU32::new(10).unwrap(),
            })
            .await
            .unwrap();

        assert_eq!(page.records.len(), 3);
        assert!(!page.has_more);
        assert_eq!(page.records[0].transit_id, all[0].transit_id);
        assert_eq!(page.records[1].transit_id, all[1].transit_id);
        assert_eq!(page.records[2].transit_id, all[2].transit_id);
    }

    #[tokio::test]
    async fn list_before_cursor() {
        let storage = test_storage().await;
        let all = seed_transit_records(&storage, 5).await;

        let page = storage
            .list_transits(TransitQuery {
                cursor: Some(all[1].stored_at),
                direction: Direction::Before,
                limit: NonZeroU32::new(10).unwrap(),
            })
            .await
            .unwrap();

        assert_eq!(page.records.len(), 3);
        assert!(!page.has_more);
        assert_eq!(page.records[0].transit_id, all[2].transit_id);
        assert_eq!(page.records[1].transit_id, all[3].transit_id);
        assert_eq!(page.records[2].transit_id, all[4].transit_id);
    }

    #[tokio::test]
    async fn list_empty() {
        let storage = test_storage().await;

        let page = storage
            .list_transits(TransitQuery {
                cursor: None,
                direction: Direction::After,
                limit: NonZeroU32::new(10).unwrap(),
            })
            .await
            .unwrap();

        assert!(page.records.is_empty());
        assert!(!page.has_more);
    }

    #[tokio::test]
    async fn list_exact_limit() {
        let storage = test_storage().await;
        seed_transit_records(&storage, 3).await;

        let page = storage
            .list_transits(TransitQuery {
                cursor: None,
                direction: Direction::After,
                limit: NonZeroU32::new(3).unwrap(),
            })
            .await
            .unwrap();

        assert_eq!(page.records.len(), 3);
        assert!(!page.has_more);
    }

    #[tokio::test]
    async fn identity_without_usage() {
        let storage = test_storage().await;

        let transit_id = Uuid::now_v7();
        storage
            .store_identities(vec![IdentityDeclaration {
                transit_id,
                provider: Provider::default(),
                header_id: Some("req-abc".into()),
                body_id: None,
            }])
            .await
            .unwrap();

        let page = storage
            .list_transits(TransitQuery {
                cursor: None,
                direction: Direction::After,
                limit: NonZeroU32::new(10).unwrap(),
            })
            .await
            .unwrap();

        assert_eq!(page.records.len(), 1);
        assert_eq!(page.records[0].transit_id, transit_id);
        assert_eq!(page.records[0].header_id.as_deref(), Some("req-abc"));
        assert!(page.records[0].model.is_none());
        assert!(page.records[0].usage.is_none());
    }

    #[tokio::test]
    async fn out_of_order_insertion_not_skipped() {
        let storage = test_storage().await;

        let early_id = Uuid::now_v7();
        let late_id = Uuid::now_v7();

        storage
            .store_identities(vec![IdentityDeclaration {
                transit_id: late_id,
                provider: Provider::default(),
                header_id: None,
                body_id: Some("late".into()),
            }])
            .await
            .unwrap();

        let page = storage
            .list_transits(TransitQuery {
                cursor: None,
                direction: Direction::Before,
                limit: NonZeroU32::new(10).unwrap(),
            })
            .await
            .unwrap();
        assert_eq!(page.records.len(), 1);
        let cursor = page.records[0].stored_at;

        storage
            .store_identities(vec![IdentityDeclaration {
                transit_id: early_id,
                provider: Provider::default(),
                header_id: None,
                body_id: Some("early".into()),
            }])
            .await
            .unwrap();

        let page = storage
            .list_transits(TransitQuery {
                cursor: Some(cursor),
                direction: Direction::After,
                limit: NonZeroU32::new(10).unwrap(),
            })
            .await
            .unwrap();
        assert_eq!(page.records.len(), 1);
        assert_eq!(page.records[0].transit_id, early_id);
    }

    #[tokio::test]
    async fn get_transits_can_backfill_usage() {
        let storage = test_storage().await;

        let transit_id = Uuid::now_v7();
        storage
            .store_identities(vec![IdentityDeclaration {
                transit_id,
                provider: Provider::OpenAI,
                header_id: Some("req-abc".into()),
                body_id: None,
            }])
            .await
            .unwrap();

        let records = storage.get_transits(vec![transit_id]).await.unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].transit_id, transit_id);
        assert!(records[0].usage.is_none());
        assert!(records[0].model.is_none());

        storage
            .store_usages(vec![UsageDeclaration {
                transit_id,
                provider: Provider::OpenAI,
                model: Some("gpt-4".into()),
                usage: json!({"input_tokens": 100, "output_tokens": 50}),
            }])
            .await
            .unwrap();

        let records = storage.get_transits(vec![transit_id]).await.unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].transit_id, transit_id);
        assert_eq!(records[0].model.as_deref(), Some("gpt-4"));
        assert!(records[0].usage.is_some());
        let usage = records[0].usage.as_ref().unwrap();
        assert_eq!(usage["input_tokens"], 100);
        assert_eq!(usage["output_tokens"], 50);
    }

    #[tokio::test]
    async fn get_transits_empty_input() {
        let storage = test_storage().await;
        let records = storage.get_transits(vec![]).await.unwrap();
        assert!(records.is_empty());
    }
}
