mod frame;
mod inspect;

use std::sync::Arc;

use axum::Extension;
use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};
use bytes::Bytes;
use conduit_core::{Config, Provider, ProviderConfig, Storages};
use futures_util::StreamExt;
use serde_json::Value;
use tokio::net::TcpListener;

use tokio::sync::mpsc;

use uuid::Uuid;

use crate::frame::Framer;
use crate::inspect::{IdentityInspector, Inspector, Report, UsageInspector, report_processor};

const MAX_REQUEST_BODY_BYTES: usize = 10 * 1024 * 1024;

// the following headers should not be forwarded upstream:
// - "host" will be wrong as it targets this proxy and is set automatically by reqwest
// - "accept-encoding" is stripped so we get raw responses for easier introspection
const STRIPPED_REQUEST_HEADERS: &[&str] = &["host", "accept-encoding"];

// the following hop-by-hop headers should not be forwarded downstream
const STRIPPED_RESPONSE_HEADERS: &[&str] = &["transfer-encoding", "connection", "keep-alive"];

pub struct AppState {
    pub http_client: reqwest::Client,
    pub report_tx: mpsc::UnboundedSender<Report>,
}

struct ProviderContext {
    name: String,
    upstream: String,
}

pub async fn start(config: Config, storages: Storages) -> anyhow::Result<()> {
    let (report_tx, report_rx) = mpsc::unbounded_channel();
    tokio::spawn(report_processor(report_rx, storages));

    let state = Arc::new(AppState {
        http_client: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300)) // TODO: make configurable, and handle better with SSE?
            .build()?,
        report_tx,
    });
    let app = build_router(&config, state);

    let listener = TcpListener::bind(&config.listen).await?;
    tracing::info!("proxy running at http://{}", listener.local_addr()?);
    axum::serve(listener, app).await?;

    Ok(())
}

pub fn build_router(config: &Config, state: Arc<AppState>) -> Router {
    let mut router = Router::new().route("/health", get(health));

    for (provider_map_key, provider_config) in &config.providers {
        router = router.nest(
            &format!("/{provider_map_key}"),
            provider_router(provider_map_key, provider_config),
        );
    }

    router.with_state(state)
}

fn provider_router(
    provider_map_key: &str,
    provider_config: &ProviderConfig,
) -> Router<Arc<AppState>> {
    let ctx = ProviderContext {
        name: provider_map_key.to_string(),
        upstream: provider_config.upstream.clone(),
    };
    Router::new()
        .fallback(any(proxy_handler))
        .layer(Extension(Arc::new(ctx)))
}

async fn proxy_handler(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<Arc<ProviderContext>>,
    client_req: Request<Body>,
) -> Response {
    let (parts, body) = client_req.into_parts();

    let path = parts.uri.path();
    let query = parts.uri.query();
    let upstream_url = match query {
        Some(q) => format!("{}{path}?{q}", context.upstream),
        None => format!("{}{path}", context.upstream),
    };

    let req_body_bytes = match axum::body::to_bytes(body, MAX_REQUEST_BODY_BYTES).await {
        Ok(bytes) => bytes,
        Err(err) => {
            tracing::error!(error = %err, "failed to read request body");
            return StatusCode::BAD_REQUEST.into_response();
        }
    };

    let mut upstream_request = state.http_client.request(parts.method, &upstream_url);
    tracing::trace!(headers = ?parts.headers, "request headers");
    for (key, value) in &parts.headers {
        if STRIPPED_REQUEST_HEADERS.contains(&key.as_str()) {
            continue;
        }
        upstream_request = upstream_request.header(key, value);
    }

    // the prime identity of _a thing_ that transits through this proxy
    let transit_id = Uuid::now_v7();
    let provider = Provider::detect(&upstream_url);

    let mut inspectors: Vec<Box<dyn Inspector>> = vec![
        Box::new(IdentityInspector::new(transit_id, provider)),
        Box::new(UsageInspector::new(transit_id, provider)),
    ];

    let request_body_as_json: Option<Value> = serde_json::from_slice(&req_body_bytes).ok();
    for inspector in &mut inspectors {
        inspector.on_request(&parts.headers, request_body_as_json.as_ref());
    }

    upstream_request = upstream_request.body(req_body_bytes);
    let upstream_response = match upstream_request.send().await {
        Ok(res) => res,
        Err(err) => {
            tracing::error!(
                provider = %context.name,
                url = %upstream_url,
                error = %err,
                "upstream request failed",
            );
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };

    let up_status = upstream_response.status();
    let up_res_headers = upstream_response.headers().clone();
    let up_body_stream = upstream_response.bytes_stream();

    tracing::trace!(headers = ?up_res_headers, "response headers");
    for inspector in &mut inspectors {
        inspector.on_response(&up_res_headers);
    }

    let is_streaming = up_res_headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|ct| ct.starts_with("text/event-stream"))
        .unwrap_or(false);

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, String>>(64);
    let provider_config_name = context.name.clone();
    let report_tx = state.report_tx.clone();
    tokio::spawn(async move {
        let mut stream = up_body_stream;

        let mut framer = if is_streaming {
            Framer::streaming()
        } else {
            Framer::unary()
        };

        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    for frame in framer.process_chunk(&bytes) {
                        tracing::trace!("chunk frame: {frame}");
                        for inspector in &mut inspectors {
                            inspector.on_frame(&frame);
                        }
                    }
                    if tx.send(Ok(bytes)).await.is_err() {
                        break;
                    }
                }
                Err(err) => {
                    tracing::error!(
                        provider = %provider_config_name,
                        error = %err,
                        "error reading upstream response chunk",
                    );
                    let _ = tx.send(Err(err.to_string())).await;
                    break;
                }
            }
        }

        for frame in framer.finish() {
            tracing::trace!("finish frame: {frame}");
            for inspector in &mut inspectors {
                inspector.on_frame(&frame);
            }
        }

        for inspector in &mut inspectors {
            for report in inspector.finish() {
                tracing::trace!("new report: {report}");
                let _ = report_tx.send(report);
            }
        }
    });

    let response_body_stream = async_stream::stream! {
        let mut rx = tokio_stream::wrappers::ReceiverStream::new(rx);
        while let Some(item) = rx.next().await {
            match item {
                Ok(bytes) => yield Ok::<_, std::io::Error>(bytes),
                Err(e) => yield Err(std::io::Error::other(e)),
            }
        }
    };

    let mut client_response = Response::builder().status(up_status.as_u16());
    for (key, value) in &up_res_headers {
        if STRIPPED_RESPONSE_HEADERS.contains(&key.as_str()) {
            continue;
        }
        client_response = client_response.header(key, value);
    }
    client_response
        .body(Body::from_stream(response_body_stream))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

async fn health() -> (StatusCode, &'static str) {
    (StatusCode::OK, "OK")
}

#[cfg(test)]
pub(crate) mod testutil {
    use super::*;

    pub fn test_state() -> Arc<AppState> {
        let (report_tx, _report_rx) = mpsc::unbounded_channel();
        Arc::new(AppState {
            http_client: reqwest::Client::new(),
            report_tx,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::test_state;
    use axum::body::Body;
    use axum::extract::Request;
    use http_body_util::BodyExt;
    use std::collections::HashMap;
    use tower::ServiceExt;

    #[tokio::test]
    async fn health_endpoint_works() -> anyhow::Result<()> {
        let config = Config::default();
        let app = build_router(&config, test_state());

        let resp = app
            .oneshot(Request::get("/health").body(Body::empty())?)
            .await?;

        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await?.to_bytes();
        assert_eq!(body, "OK");
        Ok(())
    }

    #[tokio::test]
    async fn not_found_for_unknown_routes() -> anyhow::Result<()> {
        let config = Config::default();
        let app = build_router(&config, test_state());

        let resp = app
            .oneshot(Request::get("/nonexistent").body(Body::empty())?)
            .await?;

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        Ok(())
    }

    #[tokio::test]
    async fn bad_gateway_when_upstream_unreachable() -> anyhow::Result<()> {
        let config = Config {
            listen: "127.0.0.1:0".into(),
            providers: HashMap::from([(
                "broken".into(),
                ProviderConfig {
                    upstream: "http://127.0.0.1:1".into(),
                },
            )]),
        };
        let app = build_router(&config, test_state());

        let resp = app
            .oneshot(Request::post("/broken/v1/chat/completions").body(Body::empty())?)
            .await?;

        assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
        Ok(())
    }
}

#[cfg(test)]
mod tests_openai_chat_completions {
    use super::*;
    use crate::testutil::test_state;
    use axum::body::Body;
    use axum::extract::Request;
    use http_body_util::BodyExt;
    use serde_json::json;
    use std::collections::HashMap;
    use tower::ServiceExt;

    async fn stub_app() -> Router {
        let config = Config {
            listen: "127.0.0.1:0".into(),
            providers: HashMap::from([(
                "openai".into(),
                ProviderConfig {
                    upstream: stub_upstream().await,
                },
            )]),
        };
        build_router(&config, test_state())
    }

    async fn stub_upstream() -> String {
        let app = Router::new()
            .route(
                "/v1/chat/completions",
                axum::routing::post(stub_openai_chat_completions),
            )
            .fallback(|| async { (StatusCode::NOT_FOUND, "not found") });
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async { axum::serve(listener, app).await.unwrap() });

        format!("http://{addr}")
    }

    // https://developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create
    // https://developers.openai.com/api/reference/resources/chat/subresources/completions/streaming-events
    async fn stub_openai_chat_completions(
        axum::Json(body): axum::Json<serde_json::Value>,
    ) -> Response {
        // https://developers.openai.com/api/reference/resources/completions#(resource)%20completions%20%3E%20(model)%20completion_usage%20%3E%20(schema)
        let usage = json!({
            "prompt_tokens": 19,  // number of tokens in the prompt (input)
            "completion_tokens": 10, // number of tokens in the generated completion (output)
            "total_tokens": 29,  // total number of tokens; prompt + completion
            "completion_tokens_details": { // further breakdown of tokens in completion
                "reasoning_tokens": 0,
                "audio_tokens": 0,
                "accepted_prediction_tokens": 0,
                "rejected_prediction_tokens": 0
            },
            "prompt_tokens_details": { // further breakdown of tokens in prompt
                "cached_tokens": 0,
                "audio_tokens": 0
            },
        });

        let stream_requested = body
            .get("stream")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // only streaming responses can control if usage is included or not?
        let include_usage = body
            .get("stream_options")
            .and_then(|v| v.get("include_usage"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if stream_requested {
            let make_chunk = |delta: serde_json::Value, finish_reason: Option<&str>| {
                // https://developers.openai.com/api/reference/resources/chat/subresources/completions/streaming-events#event
                let mut chunk = json!({
                    "id": "chatcmpl-test",
                    "object": "chat.completion.chunk",
                    "created": 1700000000,
                    "model": "gpt-5-mini-2025-08-07",
                    "system_fingerprint": "fp_test",
                    "choices": [{
                        "index": 0,
                        "delta": delta,
                        "logprobs": null,
                        "finish_reason": finish_reason,
                    }],
                });
                if include_usage {
                    chunk["usage"] = serde_json::Value::Null;
                }
                format!("data: {}\n\n", chunk)
            };

            let mut chunks = vec![
                make_chunk(json!({"role": "assistant", "content": ""}), None),
                make_chunk(json!({"content": "Hel"}), None),
                make_chunk(json!({"content": "lo!  How can I "}), None),
                make_chunk(json!({"content": "assist you today?"}), None),
                make_chunk(json!({}), Some("stop")),
            ];
            if include_usage {
                chunks.push(format!(
                    "data: {}\n\n",
                    json!({
                        "id": "chatcmpl-test",
                        "object": "chat.completion.chunk",
                        "created": 1700000000,
                        "model": "gpt-5-mini-2025-08-07",
                        "system_fingerprint": "fp_test",
                        "choices": [],
                        "usage": usage.clone(),
                    })
                ));
            }
            chunks.push("data: [DONE]\n\n".to_string());
            let stream = async_stream::stream! {
                for chunk in chunks {
                    yield Ok::<_, std::io::Error>(chunk);
                }
            };
            Response::builder()
                .header("content-type", "text/event-stream")
                .body(Body::from_stream(stream))
                .unwrap()
        } else {
            // non-streaming responses type usage as "optional" but it _should_ always be there
            // as the controls to include it or not are only for streaming responses
            // https://developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create
            axum::Json(json!({
              "id": "chatcmpl-test",
              "object": "chat.completion",
              "created": 1741569952,
              "model": "gpt-5.4",
              "choices": [
                {
                  "index": 0,
                  "message": {
                    "role": "assistant",
                    "content": "Hello! How can I assist you today?",
                    "refusal": null,
                    "annotations": []
                  },
                  "logprobs": null,
                  "finish_reason": "stop"
                }
              ],
              "usage": usage.clone(),
              "service_tier": "default"
            }))
            .into_response()
        }
    }

    #[tokio::test]
    async fn openai_cc_unary() -> anyhow::Result<()> {
        let app = stub_app().await;

        let resp = app
            .oneshot(
                Request::post("/openai/v1/chat/completions")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"model":"gpt-5-mini","messages":[{"role":"user","content":"Hello!"}]}"#,
                    ))?,
            )
            .await?;

        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await?.to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body)?;
        assert_eq!(json["id"], "chatcmpl-test");
        assert_eq!(json["usage"]["total_tokens"], 29);
        Ok(())
    }

    #[tokio::test]
    async fn openai_cc_streaming_without_usage() -> anyhow::Result<()> {
        let app = stub_app().await;

        let resp = app
            .oneshot(
                Request::post("/openai/v1/chat/completions")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"model":"gpt-5-mini","messages":[{"role":"user","content":"Hello!"}],"stream":true}"#))?,
            )
            .await?;

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "text/event-stream"
        );

        let body = resp.into_body().collect().await?.to_bytes();
        let body_str = std::str::from_utf8(&body)?;
        assert!(body_str.contains("\"chatcmpl-test\""));
        assert!(!body_str.contains("\"usage\""));
        assert!(body_str.ends_with("data: [DONE]\n\n"));
        Ok(())
    }

    #[tokio::test]
    async fn openai_cc_streaming_with_usage() -> anyhow::Result<()> {
        let app = stub_app().await;

        let resp = app
            .oneshot(
                Request::post("/openai/v1/chat/completions")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"model":"gpt-5-mini","messages":[{"role":"user","content":"Hello!"}],"stream":true,"stream_options":{"include_usage":true}}"#,
                    ))?,
            )
            .await?;

        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await?.to_bytes();
        let body_str = std::str::from_utf8(&body)?;

        let lines: Vec<&str> = body_str
            .lines()
            .filter(|l| l.starts_with("data: {")) // NB: termination is "data: [DONE]" so dropped
            .collect();

        for data_line in &lines[..lines.len() - 1] {
            let line = data_line.strip_prefix("data: ").unwrap();
            let json: serde_json::Value = serde_json::from_str(line)?;
            assert!(json["usage"].is_null());
        }

        let last_data = lines.last().unwrap().strip_prefix("data: ").unwrap();
        let last_json: serde_json::Value = serde_json::from_str(last_data)?;

        assert_eq!(last_json["usage"]["prompt_tokens"], 19);
        assert_eq!(last_json["usage"]["completion_tokens"], 10);
        assert_eq!(last_json["usage"]["total_tokens"], 29);

        assert!(body_str.ends_with("data: [DONE]\n\n"));
        Ok(())
    }
}
