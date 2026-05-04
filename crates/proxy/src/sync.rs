use std::time::Duration;

use conduit_core::{Config, Storages, TransitRecord};

use serde::Serialize;

// Max records sent per outgoing POST; further records are sent in subsequent
// requests within the same flush.
const SYNC_POST_BATCH_SIZE: u32 = 64;
// The cadence to flush transits to Valohai LLM.
const SYNC_FLUSH_INTERVAL: Duration = Duration::from_secs(5);

pub async fn valohai_llm_poster(storages: Storages, config: Config, http_client: reqwest::Client) {
    let api_key = &config.valohai_llm.api_key;
    let endpoint = format!("{}/api/ingest/transits/", config.valohai_llm.url);

    let mut interval = tokio::time::interval(SYNC_FLUSH_INTERVAL);

    loop {
        interval.tick().await;

        loop {
            let records = match storages.transit.list_unsent(SYNC_POST_BATCH_SIZE).await {
                Ok(r) if r.is_empty() => break,
                Ok(r) => r,
                Err(e) => {
                    tracing::error!(error = %e, "failed to list unsent transit records");
                    break;
                }
            };

            let ids: Vec<_> = records.iter().map(|r| r.transit_id).collect();
            let payload: Vec<TransitPost> = records.iter().map(TransitPost::from).collect();

            tracing::trace!(
                count = ids.len(),
                payload = %serde_json::to_string_pretty(&payload).unwrap_or_default(),
                "posting transit records",
            );

            let result = http_client
                .post(&endpoint)
                .bearer_auth(api_key)
                .json(&payload)
                .send()
                .await;

            match result {
                Ok(resp) if resp.status().is_success() => {
                    tracing::debug!(count = ids.len(), "sent transit records");
                    if let Err(e) = storages.transit.mark_sent(ids).await {
                        tracing::error!(error = %e, "failed to mark transit records as sent");
                    }
                }
                Ok(resp) => {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    tracing::warn!(
                        %status,
                        body = body.chars().take(200).collect::<String>(),
                        "endpoint rejected transit records, will retry later",
                    );
                    break;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "failed to send transit records, will retry later");
                    break;
                }
            }
        }
    }
}

#[derive(Serialize)]
struct TransitPost {
    transit_id: String,
    provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    header_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vh_headers: Option<std::collections::HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    estimated_cost_usd: Option<f64>,
}

impl From<&TransitRecord> for TransitPost {
    fn from(r: &TransitRecord) -> Self {
        Self {
            transit_id: r.transit_id.to_string(),
            provider: r.provider.to_string(),
            header_id: r.header_id.clone(),
            body_id: r.body_id.clone(),
            vh_headers: r.vh_headers.clone(),
            model: r.model.clone(),
            usage: r.usage.clone(),
            estimated_cost_usd: r.estimated_cost_usd,
        }
    }
}
