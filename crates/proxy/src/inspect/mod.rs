mod identity;
mod usage;

pub use identity::IdentityInspector;
pub use usage::UsageInspector;

use std::fmt;
use std::pin::pin;
use std::time::Duration;

use std::collections::HashMap;

use conduit_core::{IdentityDeclaration, Provider, Storages, UsageDeclaration};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::time::{Instant, sleep};
use uuid::Uuid;

use axum::http::HeaderMap;

use crate::frame::Frame;
use crate::sync::LocalOnlyCalls;

pub trait Inspector: Send {
    fn on_request(&mut self, _headers: &HeaderMap, _body_json: Option<&Value>) {}
    fn on_response(&mut self, _headers: &HeaderMap) {}
    fn on_frame(&mut self, frame: &Frame);
    fn finish(&mut self) -> Vec<Report>;
}

#[derive(Debug)]
pub struct Report {
    pub transit_id: Uuid,
    pub payload: ReportPayload,
}

#[derive(Debug)]
pub enum ReportPayload {
    Identity {
        provider: Provider,
        header_id: Option<String>,
        body_id: Option<String>,
        vh_headers: Option<HashMap<String, String>>,
    },
    Usage {
        provider: Provider,
        model: Option<String>,
        usage: Value,
    },
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Report({}, {})", self.transit_id, self.payload)
    }
}

impl fmt::Display for ReportPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReportPayload::Identity {
                provider,
                header_id,
                body_id,
                vh_headers,
            } => {
                write!(
                    f,
                    "Identity({provider:?}, header={header_id:?}, body={body_id:?}, vh={vh_headers:?})"
                )
            }
            ReportPayload::Usage {
                provider,
                model,
                usage,
            } => {
                write!(f, "Usage({provider:?}, model={model:?}, {usage})")
            }
        }
    }
}

// Flush in-memory pending buffers to storage when either the size threshold or
// the time interval is reached, whichever comes first.
const FLUSH_THRESHOLD: usize = 64;
const FLUSH_INTERVAL: Duration = Duration::from_secs(1);

pub async fn report_processor(
    mut rx: mpsc::UnboundedReceiver<Report>,
    storages: Storages,
    local_only_calls: LocalOnlyCalls,
) {
    let mut identity_deadline = pin!(sleep(FLUSH_INTERVAL));
    let mut pending_identities: Vec<IdentityDeclaration> = Vec::with_capacity(FLUSH_THRESHOLD);

    let mut usage_deadline = pin!(sleep(FLUSH_INTERVAL));
    let mut pending_usages: Vec<UsageDeclaration> = Vec::with_capacity(FLUSH_THRESHOLD);

    loop {
        tokio::select! {
            maybe_report = rx.recv() => {
                let Some(report) = maybe_report else {
                    // channel closed, flush everything pending
                    take_and_store_identities(&mut pending_identities, &storages).await;
                    take_and_store_usages(&mut pending_usages, &storages).await;
                    break;
                };

                match report.payload {
                    ReportPayload::Identity { provider, header_id, body_id, vh_headers } => {
                        // `list_unsent` never sends these; the inspector gives None
                        // for no headers, but an empty map would mean the same
                        if vh_headers.as_ref().is_none_or(HashMap::is_empty) {
                            local_only_calls.add_one();
                        }
                        pending_identities.push(IdentityDeclaration {
                            transit_id: report.transit_id,
                            provider,
                            header_id,
                            body_id,
                            vh_headers,
                        });
                    }
                    ReportPayload::Usage { provider, model, usage } => {
                        pending_usages.push(UsageDeclaration {
                            transit_id: report.transit_id,
                            provider,
                            model,
                            usage,
                        });
                    }
                }

                if pending_identities.len() >= FLUSH_THRESHOLD {
                    take_and_store_identities(&mut pending_identities, &storages).await;
                    identity_deadline.as_mut().reset(Instant::now() + FLUSH_INTERVAL);
                }
                if pending_usages.len() >= FLUSH_THRESHOLD {
                    take_and_store_usages(&mut pending_usages, &storages).await;
                    usage_deadline.as_mut().reset(Instant::now() + FLUSH_INTERVAL);
                }
            }

            // on intervals, flush the related declarations even if the batch size hasn't been reached

            _ = &mut identity_deadline => {
                take_and_store_identities(&mut pending_identities, &storages).await;
                identity_deadline.as_mut().reset(Instant::now() + FLUSH_INTERVAL);
            }
            _ = &mut usage_deadline => {
                take_and_store_usages(&mut pending_usages, &storages).await;
                usage_deadline.as_mut().reset(Instant::now() + FLUSH_INTERVAL);
            }
        }
    }
}

async fn take_and_store_identities(pending: &mut Vec<IdentityDeclaration>, storages: &Storages) {
    if pending.is_empty() {
        return;
    }

    let result = storages
        .transit
        .store_identities(std::mem::take(pending))
        .await;
    if let Err(err) = result {
        tracing::error!(error = %err, "failed to store identities");
    }
}

async fn take_and_store_usages(pending: &mut Vec<UsageDeclaration>, storages: &Storages) {
    if pending.is_empty() {
        return;
    }

    let result = storages.transit.store_usages(std::mem::take(pending)).await;
    if let Err(err) = result {
        tracing::error!(error = %err, "failed to store usages");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::FakeTransitStorage;

    fn identity(vh_headers: Option<HashMap<String, String>>) -> Report {
        Report {
            transit_id: Uuid::now_v7(),
            payload: ReportPayload::Identity {
                provider: Provider::default(),
                header_id: None,
                body_id: Some("chatcmpl-test".into()),
                vh_headers,
            },
        }
    }

    #[tokio::test]
    async fn identities_without_vh_headers_are_counted_as_local_only() {
        let (tx, rx) = mpsc::unbounded_channel();
        tx.send(identity(None)).unwrap();
        tx.send(identity(Some(HashMap::new()))).unwrap();
        tx.send(identity(Some(HashMap::from([(
            "x-vh-tag-context".to_string(),
            "support-bot".to_string(),
        )]))))
        .unwrap();
        tx.send(Report {
            transit_id: Uuid::now_v7(),
            payload: ReportPayload::Usage {
                provider: Provider::default(),
                model: None,
                usage: serde_json::json!({"total_tokens": 29}),
            },
        })
        .unwrap();
        drop(tx);

        let local_only_calls = LocalOnlyCalls::default();
        let storages = Storages {
            transit: FakeTransitStorage::with_unsent(0),
        };
        // returns once the channel is closed and drained
        report_processor(rx, storages, local_only_calls.clone()).await;

        assert_eq!(local_only_calls.take(), 2);
        assert_eq!(local_only_calls.take(), 0);
    }
}
