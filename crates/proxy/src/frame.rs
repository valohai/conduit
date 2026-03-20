use serde_json::Value;

pub struct Framer {
    mode: FramerMode,
}

enum FramerMode {
    Streaming { line_buffer: String },
    Unary { body: Vec<u8> },
}

#[derive(Debug)]
pub enum Frame {
    SseData(Value),
    UnaryResponse(Value),
}

impl std::fmt::Display for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Frame::SseData(v) => write!(f, "SseData({v})"),
            Frame::UnaryResponse(v) => write!(f, "UnaryResponse({v})"),
        }
    }
}

impl Framer {
    pub fn streaming() -> Self {
        Self {
            mode: FramerMode::Streaming {
                line_buffer: String::new(),
            },
        }
    }

    pub fn unary() -> Self {
        Self {
            mode: FramerMode::Unary { body: Vec::new() },
        }
    }

    pub fn process_chunk(&mut self, bytes: &[u8]) -> Vec<Frame> {
        match &mut self.mode {
            FramerMode::Streaming { line_buffer } => process_sse_chunk(line_buffer, bytes),
            FramerMode::Unary { body } => {
                body.extend_from_slice(bytes);
                Vec::new() // produces no frames on chunk processing
            }
        }
    }

    pub fn finish(&mut self) -> Vec<Frame> {
        match &mut self.mode {
            FramerMode::Streaming { .. } => {
                Vec::new() // produces no frames on finish
            }
            FramerMode::Unary { body } => {
                if let Ok(json) = serde_json::from_slice::<Value>(body) {
                    vec![Frame::UnaryResponse(json)]
                } else {
                    Vec::new() // produces no frames on invalid JSON
                }
            }
        }
    }
}

fn process_sse_chunk(line_buffer: &mut String, bytes: &[u8]) -> Vec<Frame> {
    let mut frames = Vec::new();

    match std::str::from_utf8(bytes) {
        Ok(text) => line_buffer.push_str(text),
        Err(e) => {
            tracing::warn!("invalid UTF-8 in SSE chunk: {e}");
            return frames;
        }
    }

    let mut start = 0;

    while let Some(pos) = line_buffer[start..].find('\n') {
        let newline_pos = start + pos;
        let line = &line_buffer[start..newline_pos];
        start = newline_pos + 1;

        // TODO: need provider-specific logic here?

        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with(':') {
            continue;
        }

        let Some(payload) = trimmed.strip_prefix("data:") else {
            continue;
        };

        if let Ok(json) = serde_json::from_str::<Value>(payload.trim_start()) {
            frames.push(Frame::SseData(json));
        }
    }

    line_buffer.drain(..start);
    frames
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn unary_buffers_then_emits_on_finish() {
        let body = json!({"id": "test", "usage": {"total_tokens": 10}});
        let bytes = serde_json::to_vec(&body).unwrap();
        let mut framer = Framer::unary();
        assert!(framer.process_chunk(&bytes).is_empty());
        let frames = framer.finish();
        assert_eq!(frames.len(), 1);
        match &frames[0] {
            Frame::UnaryResponse(v) => assert_eq!(v["id"], "test"),
            _ => panic!("expected UnaryResponse"),
        }
    }

    #[test]
    fn unary_invalid_json_emits_nothing() {
        let mut framer = Framer::unary();
        framer.process_chunk(b"not json");
        assert!(framer.finish().is_empty());
    }

    #[test]
    fn streaming_emits() {
        let mut framer = Framer::streaming();
        let frames =
            framer.process_chunk(b"data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n");
        assert_eq!(frames.len(), 1);
        match &frames[0] {
            Frame::SseData(v) => assert_eq!(v["choices"][0]["delta"]["content"], "Hi"),
            _ => panic!("expected SseData"),
        }
    }

    #[test]
    fn streaming_skips_non_json_data() {
        let mut framer = Framer::streaming();
        let frames = framer.process_chunk(b"data: [DONE]\n\n");
        assert!(frames.is_empty());
    }

    #[test]
    fn streaming_handles_split_chunks() {
        let mut framer = Framer::streaming();
        assert!(framer.process_chunk(b"data: {\"tok").is_empty());
        let frames = framer.process_chunk(b"ens\":42}\n\n");
        assert_eq!(frames.len(), 1);
        match &frames[0] {
            Frame::SseData(v) => assert_eq!(v["tokens"], 42),
            _ => panic!("expected SseData"),
        }
    }

    #[test]
    fn streaming_skips_comments_and_event_lines() {
        let mut framer = Framer::streaming();
        let frames =
            framer.process_chunk(b": this is a comment\nevent: message\ndata: {\"x\":1}\n\n");
        assert_eq!(frames.len(), 1);
        match &frames[0] {
            Frame::SseData(v) => assert_eq!(v["x"], 1),
            _ => panic!("expected SseData"),
        }
    }

    #[test]
    fn streaming_multiple_frames_in_one_chunk() {
        let mut framer = Framer::streaming();
        let frames =
            framer.process_chunk(b"data: {\"a\":1}\n\ndata: {\"b\":2}\n\ndata: [DONE]\n\n");
        assert_eq!(frames.len(), 2);
        assert!(matches!(&frames[0], Frame::SseData(v) if v["a"] == 1));
        assert!(matches!(&frames[1], Frame::SseData(v) if v["b"] == 2));
    }
}
