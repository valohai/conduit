use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use conduit_core::config::{LLM_API_KEY_ENV_VAR, ValohaiLlmConfig};
use conduit_core::{Config, Storages, TransitRecord};

use serde::Serialize;
use tracing::Level;

// Max records sent per outgoing POST; further records are sent in subsequent
// requests within the same flush.
const SYNC_POST_BATCH_SIZE: u32 = 64;
// The cadence to flush transits to Valohai LLM.
const SYNC_FLUSH_INTERVAL: Duration = Duration::from_secs(5);

// never include the api key, not even a part of it
pub fn forwarding_status(config: &ValohaiLlmConfig) -> String {
    if config.enabled() {
        format!(
            "forwarding usage to Valohai LLM at {}",
            loggable_url(&config.url)
        )
    } else {
        format!(
            "Valohai LLM forwarding off; set {LLM_API_KEY_ENV_VAR} or [valohai-llm] api_key to turn it on"
        )
    }
}

// the url is free-form config, so it may carry credentials as userinfo or in the
// query; this goes to the log at the default level, so keep only where it points
fn loggable_url(url: &str) -> String {
    // anything that doesn't parse to a host can't be trusted to have no secrets either
    let Some(mut url) = reqwest::Url::parse(url).ok().filter(|u| u.has_host()) else {
        return "<invalid url>".to_string();
    };
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    url.as_str().trim_end_matches('/').to_string()
}

// Recorded calls without X-VH- headers in this run; `list_unsent` never picks
// them, so they are counted here to tell the user why nothing gets sent.
#[derive(Clone, Default)]
pub struct LocalOnlyCalls(Arc<AtomicU64>);

impl LocalOnlyCalls {
    pub fn add(&self, count: u64) {
        self.0.fetch_add(count, Ordering::Relaxed);
    }

    pub fn take(&self) -> u64 {
        self.0.swap(0, Ordering::Relaxed)
    }
}

// the hint on tagging is worth one info line; repeating it every flush would
// bury the rest of the log under a steady stream of untagged calls
#[derive(Default)]
struct LocalOnlyNotes {
    told: bool,
}

impl LocalOnlyNotes {
    fn level(&mut self, count: u64) -> Option<Level> {
        if count == 0 {
            None
        } else if std::mem::replace(&mut self.told, true) {
            Some(Level::DEBUG)
        } else {
            Some(Level::INFO)
        }
    }
}

pub async fn valohai_llm_poster(
    storages: Storages,
    config: Config,
    http_client: reqwest::Client,
    local_only_calls: LocalOnlyCalls,
) {
    let api_key = &config.valohai_llm.api_key;
    let endpoint = format!("{}/api/ingest/transits/", config.valohai_llm.url);

    let mut interval = tokio::time::interval(SYNC_FLUSH_INTERVAL);
    let mut local_only_notes = LocalOnlyNotes::default();

    loop {
        interval.tick().await;
        // a flush that used up its time is followed by the next one right away,
        // as its tick is already due
        let sent = flush_unsent(
            &storages,
            &http_client,
            &endpoint,
            api_key,
            SYNC_FLUSH_INTERVAL,
        )
        .await;
        if sent > 0 {
            let records = if sent == 1 { "record" } else { "records" };
            tracing::info!("sent {sent} usage {records} to Valohai LLM");
        }

        let local_only = local_only_calls.take();
        let (calls, stay) = if local_only == 1 {
            ("call", "stays")
        } else {
            ("calls", "stay")
        };
        match local_only_notes.level(local_only) {
            Some(Level::INFO) => tracing::info!(
                "{local_only} recorded {calls} had no X-VH- headers and {stay} local; \
                 add an X-VH-Tag-<key> header (e.g. X-VH-Tag-Context) to forward them"
            ),
            Some(_) => tracing::debug!(
                "{local_only} more recorded {calls} had no X-VH- headers and {stay} local"
            ),
            None => {}
        }
    }
}

// one flush: posts unsent records in batches until none are left, a post fails or
// `time_limit` has passed, and returns how many records the endpoint accepted;
// the limit keeps the caller reporting while records come in as fast as they go out
async fn flush_unsent(
    storages: &Storages,
    http_client: &reqwest::Client,
    endpoint: &str,
    api_key: &str,
    time_limit: Duration,
) -> usize {
    let started = std::time::Instant::now();
    let mut sent = 0;

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
            .post(endpoint)
            .bearer_auth(api_key)
            .json(&payload)
            .send()
            .await;

        match result {
            Ok(resp) if resp.status().is_success() => {
                tracing::debug!(count = ids.len(), "sent transit records");
                sent += ids.len();
                if let Err(e) = storages.transit.mark_sent(ids).await {
                    tracing::error!(error = %e, "failed to mark transit records as sent");
                }
                if started.elapsed() >= time_limit {
                    break;
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

    sent
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::FakeTransitStorage;
    use axum::Json;
    use axum::Router;
    use axum::http::StatusCode;
    use axum::routing::post;
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    const NO_TIME_LIMIT: Duration = Duration::MAX;

    // a stand-in for the Valohai LLM ingest endpoint that answers every post with
    // `status` and keeps the size of each posted batch
    async fn ingest_endpoint(status: StatusCode) -> (String, Arc<Mutex<Vec<usize>>>) {
        let batches = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new().route(
            "/api/ingest/transits/",
            post({
                let batches = batches.clone();
                move |Json(body): Json<Vec<serde_json::Value>>| {
                    batches.lock().unwrap().push(body.len());
                    async move { status }
                }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}/api/ingest/transits/"), batches)
    }

    #[tokio::test]
    async fn flush_sends_every_unsent_record_across_batches() {
        let storage = FakeTransitStorage::with_unsent(150);
        let storages = Storages {
            transit: storage.clone(),
        };
        let (endpoint, batches) = ingest_endpoint(StatusCode::OK).await;

        let sent = flush_unsent(
            &storages,
            &reqwest::Client::new(),
            &endpoint,
            "dummy",
            NO_TIME_LIMIT,
        )
        .await;

        assert_eq!(sent, 150);
        assert_eq!(*batches.lock().unwrap(), vec![64, 64, 22]);
        assert!(storage.unsent.lock().unwrap().is_empty());
        assert_eq!(storage.sent.lock().unwrap().len(), 150);
    }

    #[tokio::test]
    async fn flush_stops_at_its_time_limit_and_leaves_the_rest_for_the_next_one() {
        let storage = FakeTransitStorage::with_unsent(150);
        let storages = Storages {
            transit: storage.clone(),
        };
        let (endpoint, batches) = ingest_endpoint(StatusCode::OK).await;

        // no time at all still lets one batch through, so a flush always makes progress
        let sent = flush_unsent(
            &storages,
            &reqwest::Client::new(),
            &endpoint,
            "dummy",
            Duration::ZERO,
        )
        .await;

        assert_eq!(sent, 64);
        assert_eq!(*batches.lock().unwrap(), vec![64]);
        assert_eq!(storage.unsent.lock().unwrap().len(), 86);
    }

    #[tokio::test]
    async fn flush_sends_nothing_when_the_endpoint_rejects() {
        let storage = FakeTransitStorage::with_unsent(150);
        let storages = Storages {
            transit: storage.clone(),
        };
        let (endpoint, batches) = ingest_endpoint(StatusCode::UNAUTHORIZED).await;

        let sent = flush_unsent(
            &storages,
            &reqwest::Client::new(),
            &endpoint,
            "dummy",
            NO_TIME_LIMIT,
        )
        .await;

        assert_eq!(sent, 0);
        // the first rejection ends the flush; the rest wait for the next one
        assert_eq!(*batches.lock().unwrap(), vec![64]);
        assert_eq!(storage.unsent.lock().unwrap().len(), 150);
        assert!(storage.sent.lock().unwrap().is_empty());
    }

    #[test]
    fn local_only_calls_are_noted_at_info_once_then_at_debug() {
        let mut notes = LocalOnlyNotes::default();
        assert_eq!(notes.level(0), None);
        assert_eq!(notes.level(3), Some(Level::INFO));
        assert_eq!(notes.level(0), None);
        assert_eq!(notes.level(2), Some(Level::DEBUG));
        assert_eq!(notes.level(5), Some(Level::DEBUG));
    }

    #[tokio::test]
    async fn flush_with_nothing_unsent_posts_nothing() {
        let storage = FakeTransitStorage::with_unsent(0);
        let storages = Storages { transit: storage };
        let (endpoint, batches) = ingest_endpoint(StatusCode::OK).await;

        let sent = flush_unsent(
            &storages,
            &reqwest::Client::new(),
            &endpoint,
            "dummy",
            NO_TIME_LIMIT,
        )
        .await;

        assert_eq!(sent, 0);
        assert!(batches.lock().unwrap().is_empty());
    }

    #[test]
    fn forwarding_status_names_the_url_but_not_the_key() {
        let config = ValohaiLlmConfig {
            api_key: "dummy-key-123".into(),
            url: "http://127.0.0.1:1234".into(),
        };
        let status = forwarding_status(&config);
        assert!(status.contains("http://127.0.0.1:1234"));
        assert!(!status.contains("dummy-key-123"));
    }

    #[test]
    fn forwarding_status_leaves_out_credentials_in_the_url() {
        let config = ValohaiLlmConfig {
            api_key: "dummy-key-123".into(),
            url: "https://someone:hunter2@llm.example.com/base?token=abc123#frag".into(),
        };
        assert_eq!(
            forwarding_status(&config),
            "forwarding usage to Valohai LLM at https://llm.example.com/base"
        );
    }

    #[test]
    fn forwarding_status_shows_the_default_url_as_configured() {
        let config = ValohaiLlmConfig {
            api_key: "dummy-key-123".into(),
            ..ValohaiLlmConfig::default()
        };
        assert_eq!(
            forwarding_status(&config),
            "forwarding usage to Valohai LLM at https://llm.valohai.com"
        );
    }

    #[test]
    fn forwarding_status_does_not_echo_a_url_it_cannot_parse() {
        let config = ValohaiLlmConfig {
            api_key: "dummy-key-123".into(),
            url: "someone:hunter2@not a url".into(),
        };
        let status = forwarding_status(&config);
        assert!(!status.contains("hunter2"), "{status}");
        assert!(status.contains("<invalid url>"), "{status}");
    }

    #[test]
    fn forwarding_status_says_how_to_turn_forwarding_on() {
        let status = forwarding_status(&ValohaiLlmConfig::default());
        assert!(status.contains(LLM_API_KEY_ENV_VAR));
        assert!(status.contains("[valohai-llm] api_key"));
    }
}
