mod usage;

pub use usage::UsageInspector;

use std::time::Duration;

use serde_json::Value;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::frame::Frame;

pub trait Inspector: Send {
    fn on_frame(&mut self, event: &Frame);
    fn finish(&mut self) -> Vec<Report>;
}

#[derive(Debug)]
pub struct Report {
    pub transit_id: Uuid,
    pub payload: ReportPayload,
}

#[derive(Debug)]
pub enum ReportPayload {
    Usage(Value),
}

pub async fn reporter(mut rx: mpsc::UnboundedReceiver<Report>) {
    let mut batch = Vec::with_capacity(64);
    loop {
        tokio::select! {
            Some(report) = rx.recv() => {
                batch.push(report);
                if batch.len() >= 64 {
                    flush(&mut batch).await;
                }
            }
            _ = tokio::time::sleep(Duration::from_secs(1)) => {
                if !batch.is_empty() {
                    flush(&mut batch).await;
                }
            }
        }
    }
}

async fn flush(batch: &mut Vec<Report>) {
    for report in batch.drain(..) {
        tracing::debug!(?report, "flushing report");
    }
}
