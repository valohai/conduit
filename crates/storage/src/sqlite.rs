use std::future::Future;
use std::pin::Pin;

use conduit_core::{Direction, UsageDeclaration, UsagePage, UsageQuery, UsageRecord, UsageStorage};
use sqlx::Row;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};

pub struct SqliteUsageStorage {
    pool: sqlx::SqlitePool,
}

impl SqliteUsageStorage {
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
            "CREATE TABLE IF NOT EXISTS usage (
                transit_id TEXT PRIMARY KEY NOT NULL,
                stored_at TEXT NOT NULL,
                usage_json TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await?;

        // index stored_at as it is used for pagination cursor
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_usage_stored_at
             ON usage (stored_at)",
        )
        .execute(&pool)
        .await?;

        tracing::debug!(%database_url, "storage initialized");

        Ok(Self { pool })
    }
}

impl UsageStorage for SqliteUsageStorage {
    fn store_usages(
        &self,
        declarations: Vec<UsageDeclaration>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
        Box::pin(async move {
            let mut tx = self.pool.begin().await?;
            for declaration in &declarations {
                let now = chrono::Utc::now();
                let stored_at = format_timestamp(&now);
                let transit_id = declaration.transit_id.to_string();
                let usage_json = declaration.usage.to_string();
                sqlx::query(
                    "INSERT INTO usage (transit_id, stored_at, usage_json) VALUES (?, ?, ?)",
                )
                .bind(&transit_id)
                .bind(&stored_at)
                .bind(&usage_json)
                .execute(&mut *tx)
                .await?;
            }
            tx.commit().await?;
            tracing::debug!(count = declarations.len(), "stored usage records");
            Ok(())
        })
    }

    fn list_usages(
        &self,
        query: UsageQuery,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<UsagePage>> + Send + '_>> {
        Box::pin(async move {
            let limit = query.limit.get();
            let fetch_limit = limit + 1; // sqlx parameter binding wants more concrete type
            let limit = limit as usize; // ... others don't

            let rows = match (&query.cursor, &query.direction) {
                (Some(cursor), Direction::Newer) => {
                    let stored_at = format_timestamp(cursor);
                    sqlx::query(
                        "SELECT transit_id, stored_at, usage_json FROM usage
                         WHERE stored_at > ?
                         ORDER BY stored_at ASC
                         LIMIT ?",
                    )
                    .bind(&stored_at)
                    .bind(fetch_limit)
                    .fetch_all(&self.pool)
                    .await?
                }
                (Some(cursor), Direction::Older) => {
                    let stored_at = format_timestamp(cursor);
                    sqlx::query(
                        "SELECT transit_id, stored_at, usage_json FROM usage
                         WHERE stored_at < ?
                         ORDER BY stored_at DESC
                         LIMIT ?",
                    )
                    .bind(&stored_at)
                    .bind(fetch_limit)
                    .fetch_all(&self.pool)
                    .await?
                }
                (None, Direction::Newer) => {
                    sqlx::query(
                        "SELECT transit_id, stored_at, usage_json FROM usage
                         ORDER BY stored_at ASC
                         LIMIT ?",
                    )
                    .bind(fetch_limit)
                    .fetch_all(&self.pool)
                    .await?
                }
                (None, Direction::Older) => {
                    sqlx::query(
                        "SELECT transit_id, stored_at, usage_json FROM usage
                         ORDER BY stored_at DESC
                         LIMIT ?",
                    )
                    .bind(fetch_limit)
                    .fetch_all(&self.pool)
                    .await?
                }
            };

            let has_more = rows.len() > limit;
            let mut records: Vec<UsageRecord> = rows
                .iter()
                .take(limit) // NB: takes one less to get the number asked for
                .map(|row| {
                    let transit_id: String = row.get("transit_id");
                    let stored_at: String = row.get("stored_at");
                    let usage_json: String = row.get("usage_json");
                    UsageRecord {
                        transit_id: transit_id.parse().unwrap(),
                        stored_at: parse_timestamp(&stored_at),
                        usage: serde_json::from_str(&usage_json).unwrap(),
                    }
                })
                .collect();

            // keep returned records in oldest-to-newest order
            if matches!(query.direction, Direction::Older) {
                records.reverse();
            }

            Ok(UsagePage { records, has_more })
        })
    }
}

fn format_timestamp(dt: &chrono::DateTime<chrono::Utc>) -> String {
    // with microseconds e.g. 2026-03-16T06:21:03.616874Z
    dt.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()
}

fn parse_timestamp(s: &str) -> chrono::DateTime<chrono::Utc> {
    s.parse::<chrono::DateTime<chrono::Utc>>().unwrap()
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;

    use super::*;
    use serde_json::json;
    use uuid::Uuid;

    async fn test_storage() -> SqliteUsageStorage {
        SqliteUsageStorage::new("sqlite::memory:").await.unwrap()
    }

    async fn seed_records(storage: &SqliteUsageStorage, count: u32) -> Vec<UsageRecord> {
        let declarations: Vec<_> = (0..count)
            .map(|_| UsageDeclaration {
                transit_id: Uuid::now_v7(),
                usage: json!({"input_tokens": 10, "output_tokens": 20}),
            })
            .collect();
        storage.store_usages(declarations).await.unwrap();

        let page = storage
            .list_usages(UsageQuery {
                cursor: None,
                direction: Direction::Newer,
                limit: NonZeroU32::new(count).unwrap(),
            })
            .await
            .unwrap();
        page.records
    }

    #[tokio::test]
    async fn list_newer_no_cursor() {
        let storage = test_storage().await;
        let seeded = seed_records(&storage, 5).await;

        let page = storage
            .list_usages(UsageQuery {
                cursor: None,
                direction: Direction::Newer,
                limit: NonZeroU32::new(3).unwrap(),
            })
            .await
            .unwrap();

        assert_eq!(page.records.len(), 3);
        assert!(page.has_more);
        assert_eq!(page.records[0].transit_id, seeded[0].transit_id);
        assert_eq!(page.records[2].transit_id, seeded[2].transit_id);
    }

    #[tokio::test]
    async fn list_newer_with_cursor() {
        let storage = test_storage().await;
        let seeded = seed_records(&storage, 5).await;

        let page = storage
            .list_usages(UsageQuery {
                cursor: Some(seeded[1].stored_at),
                direction: Direction::Newer,
                limit: NonZeroU32::new(10).unwrap(),
            })
            .await
            .unwrap();

        assert_eq!(page.records.len(), 3);
        assert!(!page.has_more);
        assert_eq!(page.records[0].transit_id, seeded[2].transit_id);
        assert_eq!(page.records[2].transit_id, seeded[4].transit_id);
    }

    #[tokio::test]
    async fn list_older_no_cursor() {
        let storage = test_storage().await;
        let seeded = seed_records(&storage, 5).await;

        let page = storage
            .list_usages(UsageQuery {
                cursor: None,
                direction: Direction::Older,
                limit: NonZeroU32::new(3).unwrap(),
            })
            .await
            .unwrap();

        assert_eq!(page.records.len(), 3);
        assert!(page.has_more);
        assert_eq!(page.records[0].transit_id, seeded[2].transit_id);
        assert_eq!(page.records[2].transit_id, seeded[4].transit_id);
    }

    #[tokio::test]
    async fn list_older_with_cursor() {
        let storage = test_storage().await;
        let seeded = seed_records(&storage, 5).await;

        let page = storage
            .list_usages(UsageQuery {
                cursor: Some(seeded[3].stored_at),
                direction: Direction::Older,
                limit: NonZeroU32::new(10).unwrap(),
            })
            .await
            .unwrap();

        assert_eq!(page.records.len(), 3);
        assert!(!page.has_more);
        assert_eq!(page.records[0].transit_id, seeded[0].transit_id);
        assert_eq!(page.records[2].transit_id, seeded[2].transit_id);
    }

    #[tokio::test]
    async fn list_empty() {
        let storage = test_storage().await;

        let page = storage
            .list_usages(UsageQuery {
                cursor: None,
                direction: Direction::Newer,
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
        seed_records(&storage, 3).await;

        let page = storage
            .list_usages(UsageQuery {
                cursor: None,
                direction: Direction::Newer,
                limit: NonZeroU32::new(3).unwrap(),
            })
            .await
            .unwrap();

        assert_eq!(page.records.len(), 3);
        assert!(!page.has_more);
    }

    #[tokio::test]
    async fn out_of_order_insertion_not_skipped() {
        let storage = test_storage().await;

        let early_id = Uuid::now_v7();
        let late_id = Uuid::now_v7();

        let late_declaration = UsageDeclaration {
            transit_id: late_id,
            usage: json!({"order": "second_to_arrive_first_to_store"}),
        };
        storage.store_usages(vec![late_declaration]).await.unwrap();

        let page = storage
            .list_usages(UsageQuery {
                cursor: None,
                direction: Direction::Older,
                limit: NonZeroU32::new(10).unwrap(),
            })
            .await
            .unwrap();
        assert_eq!(page.records.len(), 1);
        let cursor = page.records[0].stored_at;

        let early_declaration = UsageDeclaration {
            transit_id: early_id,
            usage: json!({"order": "first_to_arrive_second_to_store"}),
        };
        storage.store_usages(vec![early_declaration]).await.unwrap();

        let page = storage
            .list_usages(UsageQuery {
                cursor: Some(cursor),
                direction: Direction::Newer,
                limit: NonZeroU32::new(10).unwrap(),
            })
            .await
            .unwrap();
        assert_eq!(page.records.len(), 1);
        assert_eq!(page.records[0].transit_id, early_id);
    }
}
