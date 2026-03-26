use std::collections::HashMap;

use axum::http::HeaderMap;
use conduit_core::Provider;
use serde_json::Value;
use uuid::Uuid;

use crate::frame::Frame;
use crate::inspect::{Inspector, Report, ReportPayload};

pub struct IdentityInspector {
    transit_id: Uuid,
    provider: Provider,
    header_id: Option<String>,
    body_id: Option<String>,
    vh_headers: HashMap<String, String>,
}

impl IdentityInspector {
    pub fn new(transit_id: Uuid, provider: Provider) -> Self {
        Self {
            transit_id,
            provider,
            header_id: None,
            body_id: None,
            vh_headers: HashMap::new(),
        }
    }
}

impl Inspector for IdentityInspector {
    fn on_request(&mut self, headers: &HeaderMap, _body_json: Option<&Value>) {
        for (name, value) in headers.iter() {
            let name_str = name.as_str();
            if name_str.starts_with("x-vh-")
                && let Ok(val) = value.to_str()
            {
                self.vh_headers
                    .insert(name_str.to_string(), val.to_string());
            }
        }
    }

    fn on_response(&mut self, headers: &HeaderMap) {
        self.header_id = headers
            .get("x-request-id") // OpenAI format
            .or_else(|| headers.get("request-id")) // Anthropic format
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
    }

    fn on_frame(&mut self, frame: &Frame) {
        if self.body_id.is_some() {
            return;
        }

        let json = match frame {
            Frame::SseData(json) | Frame::UnaryResponse(json) => json,
        };

        let candidates: &[&Value] = &[
            // OpenAI Responses SSE response.created events (also exists in a few other events)
            &json["response"]["id"],
            // OpenAI Chat Completions unary body
            // OpenAI Chat Completions SSE, in all events
            // Anthropic Messages unary body
            &json["id"],
            // Anthropic Messages SSE message_start events
            &json["message"]["id"],
        ];
        for candidate in candidates {
            if let Some(id) = candidate.as_str() {
                self.body_id = Some(id.to_string());
                return;
            }
        }
    }

    fn finish(&mut self) -> Vec<Report> {
        let header_id = self.header_id.take();
        let body_id = self.body_id.take();
        let vh_headers = if self.vh_headers.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut self.vh_headers))
        };
        if header_id.is_none() && body_id.is_none() && vh_headers.is_none() {
            return vec![];
        }
        vec![Report {
            transit_id: self.transit_id,
            payload: ReportPayload::Identity {
                provider: self.provider,
                header_id,
                body_id,
                vh_headers,
            },
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::Framer;

    struct SimulationResult {
        header_id: Option<String>,
        body_id: Option<String>,
        vh_headers: Option<HashMap<String, String>>,
    }

    fn simulate_inspection(
        request_headers: Option<&HeaderMap>,
        response_headers: Option<&HeaderMap>,
        mut framer: Framer,
        response_chunks: &[&[u8]],
    ) -> SimulationResult {
        let mut inspector = IdentityInspector::new(Uuid::nil(), Provider::default());
        if let Some(req_headers) = request_headers {
            inspector.on_request(req_headers, None);
        }
        if let Some(res_headers) = response_headers {
            inspector.on_response(res_headers);
        }
        for chunk in response_chunks {
            for frame in framer.process_chunk(chunk) {
                inspector.on_frame(&frame);
            }
        }
        for frame in framer.finish() {
            inspector.on_frame(&frame);
        }
        let reports = inspector.finish();
        match reports.first() {
            Some(Report {
                payload:
                    ReportPayload::Identity {
                        header_id,
                        body_id,
                        vh_headers,
                        ..
                    },
                ..
            }) => SimulationResult {
                header_id: header_id.clone(),
                body_id: body_id.clone(),
                vh_headers: vh_headers.clone(),
            },
            _ => SimulationResult {
                header_id: None,
                body_id: None,
                vh_headers: None,
            },
        }
    }

    #[test]
    fn header_id_from_x_request_id() {
        let mut res_headers = HeaderMap::new();
        res_headers.insert("x-request-id", "abc-123".parse().unwrap());
        let sim = simulate_inspection(None, Some(&res_headers), Framer::unary(), &[]);
        assert_eq!(sim.header_id.as_deref(), Some("abc-123"));
    }

    #[test]
    fn header_id_from_request_id() {
        let mut res_headers = HeaderMap::new();
        res_headers.insert("request-id", "def-456".parse().unwrap());
        let sim = simulate_inspection(None, Some(&res_headers), Framer::unary(), &[]);
        assert_eq!(sim.header_id.as_deref(), Some("def-456"));
    }

    #[test]
    fn header_id_prefers_x_request_id() {
        let mut res_headers = HeaderMap::new();
        res_headers.insert("x-request-id", "from-x".parse().unwrap());
        res_headers.insert("request-id", "from-plain".parse().unwrap());
        let sim = simulate_inspection(None, Some(&res_headers), Framer::unary(), &[]);
        assert_eq!(sim.header_id.as_deref(), Some("from-x"));
    }

    #[test]
    fn header_id_none_when_missing() {
        let res_headers = HeaderMap::new();
        let sim = simulate_inspection(None, Some(&res_headers), Framer::unary(), &[]);
        assert_eq!(sim.header_id, None);
        assert_eq!(sim.body_id, None);
    }

    #[test]
    fn body_id_from_unary_openai_cc() {
        let body =
            b"{\"id\":\"chatcmpl-B9MBs8CjcvOU2jLn4n570S5qMJKcT\",\"object\":\"chat.completion\"}";
        let sim = simulate_inspection(None, None, Framer::unary(), &[&body[..]]);
        assert_eq!(
            sim.body_id.as_deref(),
            Some("chatcmpl-B9MBs8CjcvOU2jLn4n570S5qMJKcT")
        );
    }

    #[test]
    fn body_id_from_streaming_openai_cc() {
        let chunks: Vec<&[u8]> = vec![
            b"data: {\"id\":\"chatcmpl-test123\",\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n",
            b"data: {\"id\":\"chatcmpl-test123\",\"choices\":[],\"usage\":{\"total_tokens\":10}}\n\n",
            b"data: [DONE]\n\n",
        ];
        let sim = simulate_inspection(None, None, Framer::streaming(), &chunks);
        assert_eq!(sim.body_id.as_deref(), Some("chatcmpl-test123"));
    }

    #[test]
    fn body_id_from_unary_openai_responses() {
        let body = b"{\"id\":\"resp_67cb61fa3a448190bcf2c42d96f0d1a8\",\"object\":\"response\"}";
        let sim = simulate_inspection(None, None, Framer::unary(), &[&body[..]]);
        assert_eq!(
            sim.body_id.as_deref(),
            Some("resp_67cb61fa3a448190bcf2c42d96f0d1a8")
        );
    }

    #[test]
    fn body_id_from_streaming_openai_responses() {
        let chunks: Vec<&[u8]> = vec![
            b"event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_abc123\",\"status\":\"in_progress\"}}\n\n",
            b"event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"Hi\"}\n\n",
            b"event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_abc123\",\"status\":\"completed\",\"usage\":{\"input_tokens\":10,\"output_tokens\":5}}}\n\n",
        ];
        let sim = simulate_inspection(None, None, Framer::streaming(), &chunks);
        assert_eq!(sim.body_id.as_deref(), Some("resp_abc123"));
    }

    #[test]
    fn body_id_from_unary_anthropic() {
        let body = b"{\"id\":\"msg_01XFDUDYJgAACzvnptvVoYEL\",\"type\":\"message\",\"role\":\"assistant\"}";
        let sim = simulate_inspection(None, None, Framer::unary(), &[&body[..]]);
        assert_eq!(sim.body_id.as_deref(), Some("msg_01XFDUDYJgAACzvnptvVoYEL"));
    }

    #[test]
    fn body_id_from_streaming_anthropic() {
        let chunks: Vec<&[u8]> = vec![
            b"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_test456\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[]}}\n\n",
            b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n",
            b"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        ];
        let sim = simulate_inspection(None, None, Framer::streaming(), &chunks);
        assert_eq!(sim.body_id.as_deref(), Some("msg_test456"));
    }

    #[test]
    fn body_id_none_when_no_id_in_body() {
        let body = b"{\"choices\":[],\"usage\":{\"total_tokens\":5}}";
        let sim = simulate_inspection(None, None, Framer::unary(), &[&body[..]]);
        assert_eq!(sim.header_id, None);
        assert_eq!(sim.body_id, None);
    }

    #[test]
    fn body_id_prefers_first_found() {
        let chunks: Vec<&[u8]> = vec![
            b"data: {\"id\":\"chatcmpl-first\",\"choices\":[{\"delta\":{\"content\":\"a\"}}]}\n\n",
            b"data: {\"id\":\"chatcmpl-second\",\"choices\":[{\"delta\":{\"content\":\"b\"}}]}\n\n",
            b"data: [DONE]\n\n",
        ];
        let sim = simulate_inspection(None, None, Framer::streaming(), &chunks);
        assert_eq!(sim.body_id.as_deref(), Some("chatcmpl-first"));
    }

    #[test]
    fn vh_headers_none_when_absent() {
        let req_headers = HeaderMap::new();
        let sim = simulate_inspection(Some(&req_headers), None, Framer::unary(), &[]);
        assert_eq!(sim.vh_headers, None);
    }

    #[test]
    fn vh_headers_extracted() {
        let mut req_headers = HeaderMap::new();
        req_headers.insert("x-vh-trace-id", "trace-abc".parse().unwrap());
        req_headers.insert("x-vh-run-id", "run-123".parse().unwrap());
        req_headers.insert("x-vh-future", "{\"a\": \"b\"}".parse().unwrap());
        let sim = simulate_inspection(Some(&req_headers), None, Framer::unary(), &[]);
        let vh_headers = sim.vh_headers.unwrap();
        assert_eq!(vh_headers.get("x-vh-trace-id").unwrap(), "trace-abc");
        assert_eq!(vh_headers.get("x-vh-run-id").unwrap(), "run-123");
        assert_eq!(vh_headers.get("x-vh-future").unwrap(), "{\"a\": \"b\"}");
    }

    #[test]
    fn vh_headers_ignores_non_x_vh() {
        let mut req_headers = HeaderMap::new();
        req_headers.insert("x-other-header", "value".parse().unwrap());
        req_headers.insert("x-vhh-header", "value".parse().unwrap());
        req_headers.insert("vh-unrelated", "value".parse().unwrap());
        req_headers.insert("x-vh-trace-id", "trace-abc".parse().unwrap());
        let sim = simulate_inspection(Some(&req_headers), None, Framer::unary(), &[]);
        let vh_headers = sim.vh_headers.unwrap();
        assert_eq!(vh_headers.len(), 1);
        assert_eq!(vh_headers.get("x-vh-trace-id").unwrap(), "trace-abc");
    }

    #[test]
    fn vh_headers_go_with_identity() {
        let mut req_headers = HeaderMap::new();
        req_headers.insert("x-vh-trace-id", "trace-abc".parse().unwrap());
        let mut resp_headers = HeaderMap::new();
        resp_headers.insert("x-request-id", "req-xyz".parse().unwrap());
        let body = b"{\"id\":\"chatcmpl-test\",\"object\":\"chat.completion\"}";
        let sim = simulate_inspection(
            Some(&req_headers),
            Some(&resp_headers),
            Framer::unary(),
            &[&body[..]],
        );
        assert_eq!(sim.header_id.as_deref(), Some("req-xyz"));
        assert_eq!(sim.body_id.as_deref(), Some("chatcmpl-test"));
        let vh_headers = sim.vh_headers.unwrap();
        assert_eq!(vh_headers.get("x-vh-trace-id").unwrap(), "trace-abc");
    }
}
