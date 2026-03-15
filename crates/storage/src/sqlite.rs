use std::future::Future;
use std::pin::Pin;

use conduit_core::{Direction, UsagePage, UsageQuery, UsageRecord, UsageStorage};
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
                usage_json TEXT NOT NULL
            )",
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
        records: Vec<UsageRecord>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
        Box::pin(async move {
            let mut tx = self.pool.begin().await?;
            for record in &records {
                let transit_id = record.transit_id.to_string();
                let usage_json = record.usage.to_string();
                sqlx::query("INSERT INTO usage (transit_id, usage_json) VALUES (?, ?)")
                    .bind(&transit_id)
                    .bind(&usage_json)
                    .execute(&mut *tx)
                    .await?;
            }
            tx.commit().await?;
            tracing::debug!(count = records.len(), "stored usage records");
            Ok(())
        })
    }

    fn list_usages(
        &self,
        query: UsageQuery,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<UsagePage>> + Send + '_>> {
        Box::pin(async move {
            let limit = query.limit.get();
            let fetch_limit = limit + 1;
            let rows = match (&query.cursor, &query.direction) {
                (Some(cursor), Direction::Newer) => {
                    let cursor = cursor.to_string();
                    sqlx::query(
                        "SELECT transit_id, usage_json FROM usage
                         WHERE transit_id > ? ORDER BY transit_id ASC LIMIT ?",
                    )
                    .bind(&cursor)
                    .bind(fetch_limit)
                    .fetch_all(&self.pool)
                    .await?
                }
                (Some(cursor), Direction::Older) => {
                    let cursor = cursor.to_string();
                    sqlx::query(
                        "SELECT transit_id, usage_json FROM usage
                         WHERE transit_id < ? ORDER BY transit_id DESC LIMIT ?",
                    )
                    .bind(&cursor)
                    .bind(fetch_limit)
                    .fetch_all(&self.pool)
                    .await?
                }
                (None, Direction::Newer) => {
                    sqlx::query(
                        "SELECT transit_id, usage_json FROM usage
                         ORDER BY transit_id ASC LIMIT ?",
                    )
                    .bind(fetch_limit)
                    .fetch_all(&self.pool)
                    .await?
                }
                (None, Direction::Older) => {
                    sqlx::query(
                        "SELECT transit_id, usage_json FROM usage
                         ORDER BY transit_id DESC LIMIT ?",
                    )
                    .bind(fetch_limit)
                    .fetch_all(&self.pool)
                    .await?
                }
            };

            let has_more = rows.len() > limit as usize;
            let mut records: Vec<UsageRecord> = rows
                .iter()
                .take(limit as usize)
                .map(|row| {
                    let transit_id: String = row.get("transit_id");
                    let usage_json: String = row.get("usage_json");
                    UsageRecord {
                        transit_id: transit_id.parse().unwrap(),
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

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;

    use super::*;
    use serde_json::json;
    use uuid::Uuid;

    async fn test_storage() -> SqliteUsageStorage {
        SqliteUsageStorage::new("sqlite::memory:").await.unwrap()
    }

    fn make_record(transit_id: Uuid) -> UsageRecord {
        UsageRecord {
            transit_id,
            usage: json!({"input_tokens": 10, "output_tokens": 20}),
        }
    }

    async fn seed_records(storage: &SqliteUsageStorage, count: usize) -> Vec<Uuid> {
        let mut ids = Vec::new();
        for _ in 0..count {
            let id = Uuid::now_v7();
            ids.push(id);
        }
        let records: Vec<UsageRecord> = ids.iter().map(|id| make_record(*id)).collect();
        storage.store_usages(records).await.unwrap();
        ids
    }

    #[tokio::test]
    async fn list_newer_no_cursor() {
        let storage = test_storage().await;
        let ids = seed_records(&storage, 5).await;

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
        assert_eq!(page.records[0].transit_id, ids[0]);
        assert_eq!(page.records[2].transit_id, ids[2]);
    }

    #[tokio::test]
    async fn list_newer_with_cursor() {
        let storage = test_storage().await;
        let ids = seed_records(&storage, 5).await;

        let page = storage
            .list_usages(UsageQuery {
                cursor: Some(ids[1]),
                direction: Direction::Newer,
                limit: NonZeroU32::new(10).unwrap(),
            })
            .await
            .unwrap();

        assert_eq!(page.records.len(), 3);
        assert!(!page.has_more);
        assert_eq!(page.records[0].transit_id, ids[2]);
        assert_eq!(page.records[2].transit_id, ids[4]);
    }

    #[tokio::test]
    async fn list_older_no_cursor() {
        let storage = test_storage().await;
        let ids = seed_records(&storage, 5).await;

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
        assert_eq!(page.records[0].transit_id, ids[2]);
        assert_eq!(page.records[2].transit_id, ids[4]);
    }

    #[tokio::test]
    async fn list_older_with_cursor() {
        let storage = test_storage().await;
        let ids = seed_records(&storage, 5).await;

        let page = storage
            .list_usages(UsageQuery {
                cursor: Some(ids[3]),
                direction: Direction::Older,
                limit: NonZeroU32::new(10).unwrap(),
            })
            .await
            .unwrap();

        assert_eq!(page.records.len(), 3);
        assert!(!page.has_more);
        assert_eq!(page.records[0].transit_id, ids[0]);
        assert_eq!(page.records[2].transit_id, ids[2]);
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
}
