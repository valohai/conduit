use serde_json::Value;

use crate::frame::Frame;
use crate::inspect::{Inspector, Report};

pub struct UsageInspector {
    usage: Option<Value>,
}

impl UsageInspector {
    pub fn new() -> Self {
        Self { usage: None }
    }
}

impl Inspector for UsageInspector {
    fn on_frame(&mut self, frame: &Frame) {
        tracing::trace!("on_frame: {:?}", frame);
        match frame {
            Frame::SseData(json) | Frame::UnaryResponse(json) => merge_usage(&mut self.usage, json),
        }
    }

    fn finish(&mut self) -> Vec<Report> {
        tracing::trace!("finish: {:?}", self.usage);
        self.usage.take().map(Report::Usage).into_iter().collect()
    }
}

fn merge_usage(usage: &mut Option<Value>, payload: &Value) {
    let candidates: &[&Value] = &[
        // OpenAI Chat Completions unary and SSE
        // Anthropic Messages unary
        // Anthropic Messages SSE message_delta event
        &payload["usage"],
        // Anthropic Messages SSE message_start event
        &payload["message"]["usage"],
        // OpenAI Responses SSE response.completed event
        &payload["response"]["usage"],
    ];
    for candidate in candidates {
        let Some(obj) = candidate.as_object() else {
            continue;
        };
        let usage_map = usage
            .get_or_insert_with(|| Value::Object(serde_json::Map::new()))
            .as_object_mut()
            .expect("usage is always an Object");
        usage_map.extend(
            obj.iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k.clone(), v.clone())),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::Framer;
    use serde_json::json;

    fn extract_usage(framer: &mut Framer, chunks: &[&[u8]]) -> Option<Value> {
        let mut inspector = UsageInspector::new();
        for chunk in chunks {
            for frame in framer.process_chunk(chunk) {
                inspector.on_frame(&frame);
            }
        }
        for frame in framer.finish() {
            inspector.on_frame(&frame);
        }
        inspector
            .finish()
            .into_iter()
            .map(|Report::Usage(v)| v)
            .next()
    }

    fn extract_usage_unary(body: &Value) -> Option<Value> {
        let bytes = serde_json::to_vec(body).unwrap();
        let mut framer = Framer::unary();
        extract_usage(&mut framer, &[&bytes])
    }

    fn extract_usage_streaming(chunks: &[&[u8]]) -> Option<Value> {
        let mut framer = Framer::streaming();
        extract_usage(&mut framer, chunks)
    }

    #[test]
    fn unary_no_usage() {
        let body = json!({"id": "chatcmpl-test"});
        assert!(extract_usage_unary(&body).is_none());
    }

    #[test]
    fn streaming_no_usage() {
        assert!(
            extract_usage_streaming(&[
                b"data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n",
                b"data: [DONE]\n\n",
            ])
            .is_none()
        );
    }

    #[test]
    fn unary_openai_cc_usage() {
        // https://developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create
        let body = json!({
            "id": "chatcmpl-test",
            "usage": {
              "prompt_tokens": 19,
              "completion_tokens": 10,
              "total_tokens": 29,
              "prompt_tokens_details": {
                "cached_tokens": 0,
                "audio_tokens": 0
              },
              "completion_tokens_details": {
                "reasoning_tokens": 0,
                "audio_tokens": 0,
                "accepted_prediction_tokens": 0,
                "rejected_prediction_tokens": 0
              }
            },
        });
        let usage = extract_usage_unary(&body).unwrap();
        assert_eq!(usage["prompt_tokens"], 19);
        assert_eq!(usage["completion_tokens"], 10);
        assert_eq!(usage["total_tokens"], 29);
    }

    #[test]
    fn streaming_openai_cc_usage() {
        let usage = extract_usage_streaming(&[
            b"data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n",
            b"data: {\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5,\"total_tokens\":15}}\n\n",
            b"data: [DONE]\n\n",
        ])
        .unwrap();
        assert_eq!(usage["prompt_tokens"], 10);
        assert_eq!(usage["completion_tokens"], 5);
        assert_eq!(usage["total_tokens"], 15);
    }

    #[test]
    fn streaming_skips_null_usages() {
        // OpenAI Chat Completions API has null usages in the intermediate events
        let usage = extract_usage_streaming(&[
            b"data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}],\"usage\":null}\n\n",
            b"data: {\"choices\":[],\"usage\":{\"total_tokens\":29}}\n\n",
            b"data: [DONE]\n\n",
        ])
        .unwrap();
        assert_eq!(usage["total_tokens"], 29);
    }

    #[test]
    fn streaming_handles_chunks_split_across_boundaries() {
        let usage = extract_usage_streaming(&[
            b"data: {\"choices\":[],\"usa",
            b"ge\":{\"total_tokens\":42}}\n\ndata: [DONE]\n\n",
        ])
        .unwrap();
        assert_eq!(usage["total_tokens"], 42);
    }

    #[test]
    fn streaming_ignores_non_data_fields() {
        let usage = extract_usage_streaming(&[
            b": this is a comment\n",
            b"event: message\n",
            b"data: {\"choices\":[],\"usage\":{\"total_tokens\":7}}\n\n",
            b"data: [DONE]\n\n",
        ])
        .unwrap();
        assert_eq!(usage["total_tokens"], 7);
    }

    #[test]
    fn unary_openai_responses_usage() {
        // https://developers.openai.com/api/reference/resources/responses/methods/create
        let body = json!({
            "id": "resp_test123",
            "usage": {
              "input_tokens": 36,
              "input_tokens_details": {
                "cached_tokens": 10
              },
              "output_tokens": 87,
              "output_tokens_details": {
                "reasoning_tokens": 20
              },
              "total_tokens": 123
            },
        });
        let usage = extract_usage_unary(&body).unwrap();
        assert_eq!(usage["input_tokens"], 36);
        assert_eq!(usage["output_tokens"], 87);
        assert_eq!(usage["total_tokens"], 123);
        assert_eq!(usage["input_tokens_details"]["cached_tokens"], 10);
        assert_eq!(usage["output_tokens_details"]["reasoning_tokens"], 20);
    }

    #[test]
    fn streaming_openai_responses_usage() {
        // https://developers.openai.com/api/reference/resources/responses/methods/create (under Streaming tab)
        let chunks: Vec<&[u8]> = vec![
            b"event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_67c9fdcecf488190bdd9a0409de3a1ec07b8b0ad4e5eb654\",\"object\":\"response\",\"created_at\":1741290958,\"status\":\"in_progress\",\"usage\":null}}\n\n",
            b"event: response.in_progress\ndata: {\"type\":\"response.in_progress\",\"response\":{\"id\":\"resp_67c9fdcecf488190bdd9a0409de3a1ec07b8b0ad4e5eb654\",\"object\":\"response\",\"status\":\"in_progress\",\"usage\":null}}\n\n",
            b"event: response.output_item.added\ndata: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"id\":\"msg_67c9fdcf37fc8190ba82116e33fb28c507b8b0ad4e5eb654\",\"type\":\"message\",\"status\":\"in_progress\",\"role\":\"assistant\",\"content\":[]}}\n\n",
            b"event: response.content_part.added\ndata: {\"type\":\"response.content_part.added\",\"item_id\":\"msg_67c9fdcf37fc8190ba82116e33fb28c507b8b0ad4e5eb654\",\"output_index\":0,\"content_index\":0,\"part\":{\"type\":\"output_text\",\"text\":\"\",\"annotations\":[]}}\n\n",
            b"event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"item_id\":\"msg_67c9fdcf37fc8190ba82116e33fb28c507b8b0ad4e5eb654\",\"output_index\":0,\"content_index\":0,\"delta\":\"Hi\"}\n\n",
            b"event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"item_id\":\"msg_67c9fdcf37fc8190ba82116e33fb28c507b8b0ad4e5eb654\",\"output_index\":0,\"content_index\":0,\"delta\":\" there! How can I assist you today?\"}\n\n",
            b"event: response.output_text.done\ndata: {\"type\":\"response.output_text.done\",\"item_id\":\"msg_67c9fdcf37fc8190ba82116e33fb28c507b8b0ad4e5eb654\",\"output_index\":0,\"content_index\":0,\"text\":\"Hi there! How can I assist you today?\"}\n\n",
            b"event: response.content_part.done\ndata: {\"type\":\"response.content_part.done\",\"item_id\":\"msg_67c9fdcf37fc8190ba82116e33fb28c507b8b0ad4e5eb654\",\"output_index\":0,\"content_index\":0,\"part\":{\"type\":\"output_text\",\"text\":\"Hi there! How can I assist you today?\",\"annotations\":[]}}\n\n",
            b"event: response.output_item.done\ndata: {\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"id\":\"msg_67c9fdcf37fc8190ba82116e33fb28c507b8b0ad4e5eb654\",\"type\":\"message\",\"status\":\"completed\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"Hi there! How can I assist you today?\",\"annotations\":[]}]}}\n\n",
            b"event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_67c9fdcecf488190bdd9a0409de3a1ec07b8b0ad4e5eb654\",\"object\":\"response\",\"created_at\":1741290958,\"status\":\"completed\",\"usage\":{\"input_tokens\":37,\"output_tokens\":11,\"output_tokens_details\":{\"reasoning_tokens\":0},\"total_tokens\":48}}}\n\n",
        ];
        let usage = extract_usage_streaming(&chunks).unwrap();
        assert_eq!(usage["input_tokens"], 37);
        assert_eq!(usage["output_tokens"], 11);
        assert_eq!(usage["total_tokens"], 48);
        assert_eq!(usage["output_tokens_details"]["reasoning_tokens"], 0);
    }

    #[test]
    fn unary_anthropic_messages_usage() {
        // https://platform.claude.com/docs/en/api/messages/create
        let body = json!({
          "id": "msg_test123",
          "usage": {
            "service_tier": "standard",
            "input_tokens": 2095,
            "output_tokens": 503,
            "cache_creation_input_tokens":  2020,
            "cache_read_input_tokens": 3030,
            "inference_geo": "inference_geo",
            "server_tool_use": {
              "web_fetch_requests": 2,
              "web_search_requests": 0
            },
            "cache_creation": {
              "ephemeral_1h_input_tokens": 0,
              "ephemeral_5m_input_tokens": 0
            },
          }
        });
        let usage = extract_usage_unary(&body).unwrap();
        assert_eq!(usage["input_tokens"], 2095);
        assert_eq!(usage["output_tokens"], 503);
        assert_eq!(usage["cache_creation_input_tokens"], 2020);
        assert_eq!(usage["cache_read_input_tokens"], 3030);
        assert_eq!(usage["server_tool_use"]["web_fetch_requests"], 2);
        assert_eq!(usage["server_tool_use"]["web_search_requests"], 0);
        assert_eq!(usage["service_tier"], "standard");
    }

    #[test]
    fn streaming_anthropic_messages_basic() {
        // https://platform.claude.com/docs/en/build-with-claude/streaming#basic-streaming-request
        let chunks: Vec<&[u8]> = vec![
            b"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_test123\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-opus-4-6\",\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":25,\"output_tokens\":1}}}\n\n",
            b"event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            b"event: ping\ndata: {\"type\":\"ping\"}\n\n",
            b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n",
            b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"!\"}}\n\n",
            b"event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            b"event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":15}}\n\n",
            b"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        ];
        let usage = extract_usage_streaming(&chunks).unwrap();
        assert_eq!(usage["input_tokens"], 25);
        assert_eq!(usage["output_tokens"], 15);
    }

    #[test]
    fn streaming_anthropic_messages_tool_use() {
        // https://platform.claude.com/docs/en/build-with-claude/streaming#streaming-request-with-web-search-tool-use
        let chunks: Vec<&[u8]> = vec![
            b"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_01G\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-opus-4-6\",\"content\":[],\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":2679,\"cache_creation_input_tokens\":0,\"cache_read_input_tokens\":0,\"output_tokens\":3}}}\n\n",
            b"event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"I'll check\"}}\n\n",
            b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\" the current weather in New York City for you\"}}\n\n",
            b"event: ping\ndata: {\"type\": \"ping\"}\n\n",
            b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\".\"}}\n\n",
            b"event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            b"event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"server_tool_use\",\"id\":\"srvtoolu_014hJH\",\"name\":\"web_search\",\"input\":{}}}\n\n",
            b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"\"}}\n\n",
            b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"query\"}}\n\n",
            b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"\\\":\"}}\n\n",
            b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\" \\\"weather\"}}\n\n",
            b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\" NYC today\\\"}\"}}\n\n",
            b"event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
            b"event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":2,\"content_block\":{\"type\":\"web_search_tool_result\",\"tool_use_id\":\"srvtoolu_014hJH\",\"content\":[{\"type\":\"web_search_result\",\"title\":\"Weather in NYC\",\"url\":\"https://example.com/weather\",\"encrypted_content\":\"Ev0D...\",\"page_age\":null}]}}\n\n",
            b"event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":2}\n\n",
            b"event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":3,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":3,\"delta\":{\"type\":\"text_delta\",\"text\":\"Here's the current weather information for New York\"}}\n\n",
            b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":3,\"delta\":{\"type\":\"text_delta\",\"text\":\" City.\"}}\n\n",
            b"event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":3}\n\n",
            b"event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"input_tokens\":10682,\"cache_creation_input_tokens\":0,\"cache_read_input_tokens\":0,\"output_tokens\":510,\"server_tool_use\":{\"web_search_requests\":1}}}\n\n",
            b"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        ];
        let usage = extract_usage_streaming(&chunks).unwrap();
        assert_eq!(usage["input_tokens"], 10682);
        assert_eq!(usage["output_tokens"], 510);
        assert_eq!(usage["server_tool_use"]["web_search_requests"], 1);
    }
}
