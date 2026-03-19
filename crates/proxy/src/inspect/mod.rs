mod identity;
mod usage;

pub use identity::IdentityInspector;
pub use usage::UsageInspector;

use std::pin::pin;
use std::time::Duration;

use conduit_core::{Storages, UsageDeclaration};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::time::{Instant, sleep};
use uuid::Uuid;

use axum::http::HeaderMap;

use crate::frame::Frame;

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
        header_id: Option<String>,
        body_id: Option<String>,
    },
    Usage {
        model: Option<String>,
        usage: Value,
    },
}

const BATCH_SIZE: usize = 64;
const FLUSH_INTERVAL: Duration = Duration::from_secs(1);

pub async fn report_processor(mut rx: mpsc::UnboundedReceiver<Report>, storages: Storages) {
    let mut pending_usages: Vec<UsageDeclaration> = Vec::with_capacity(BATCH_SIZE);
    let mut usage_deadline = pin!(sleep(FLUSH_INTERVAL));
    loop {
        tokio::select! {
            maybe_report = rx.recv() => {
                let Some(report) = maybe_report else {
                    // channel closed, flush everything pending
                    store_pending_usages(&mut pending_usages, &storages).await;
                    break;
                };

                match report.payload {
                    ReportPayload::Identity { .. } => todo!("store identity reports"),
                    ReportPayload::Usage { model, usage } => {
                        pending_usages.push(UsageDeclaration {
                            transit_id: report.transit_id,
                            model,
                            usage,
                        });
                    }
                }

                if let Some(usages) = take_if_pending_usages_full(&mut pending_usages) {
                    store_usages(usages, &storages).await;
                    usage_deadline.as_mut().reset(Instant::now() + FLUSH_INTERVAL);
                }
            }
            _ = &mut usage_deadline => {
                // flush usages on interval
                store_pending_usages(&mut pending_usages, &storages).await;
                usage_deadline.as_mut().reset(Instant::now() + FLUSH_INTERVAL);
            }
        }
    }
}

async fn store_pending_usages(pending_usage: &mut Vec<UsageDeclaration>, storages: &Storages) {
    if !pending_usage.is_empty() {
        store_usages(std::mem::take(pending_usage), storages).await;
    }
}

async fn store_usages(declarations: Vec<UsageDeclaration>, storages: &Storages) {
    if let Err(err) = storages.usage.store_usages(declarations).await {
        tracing::error!(error = %err, "failed to store usages");
    }
}

fn take_if_pending_usages_full(batch: &mut Vec<UsageDeclaration>) -> Option<Vec<UsageDeclaration>> {
    if batch.len() >= BATCH_SIZE {
        Some(std::mem::take(batch))
    } else {
        None
    }
}
