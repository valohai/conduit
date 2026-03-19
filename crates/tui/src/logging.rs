use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Mutex};

use chrono::Local;
use tracing::field::{Field, Visit};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

const MAX_LOG_LINES: usize = 5;

pub type LogBuffer = Arc<Mutex<VecDeque<String>>>;

pub fn new_log_buffer() -> LogBuffer {
    Arc::new(Mutex::new(VecDeque::new()))
}

pub struct TuiLogLayer {
    buffer: LogBuffer,
}

impl TuiLogLayer {
    pub fn new(buffer: LogBuffer) -> Self {
        Self { buffer }
    }
}

impl<S: tracing::Subscriber> Layer<S> for TuiLogLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();
        let level = metadata.level();
        let target = metadata.target();

        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        let message = visitor.finish();

        let timestamp = Local::now().format("%H:%M:%S");
        let line = format!("{timestamp} {level} {target}: {message}");

        if let Ok(mut buf) = self.buffer.lock() {
            buf.push_back(line);
            while buf.len() > MAX_LOG_LINES {
                buf.pop_front();
            }
        }
    }
}

#[derive(Default)]
struct MessageVisitor {
    message: Option<String>,
    fields: Vec<String>,
}

impl MessageVisitor {
    fn finish(self) -> String {
        let msg = self.message.unwrap_or_default();
        if self.fields.is_empty() {
            msg
        } else {
            format!("{msg} {}", self.fields.join(", "))
        }
    }
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{:?}", value));
        } else {
            self.fields.push(format!("{}={:?}", field.name(), value));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        } else {
            self.fields.push(format!("{}={}", field.name(), value));
        }
    }
}
