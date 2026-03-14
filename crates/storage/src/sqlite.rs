use std::future::Future;
use std::pin::Pin;

use conduit_core::{UsageRecord, UsageStorage};
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
}
