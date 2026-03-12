use std::sync::Arc;

use axum::Extension;
use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};
use bytes::Bytes;
use conduit_core::{Config, ProviderConfig};
use futures_util::StreamExt;
use tokio::net::TcpListener;

const MAX_REQUEST_BODY_BYTES: usize = 10 * 1024 * 1024;

// the following headers should not be forwarded upstream:
// - "host" will be wrong as it targets this proxy and is set automatically by reqwest
// - "accept-encoding" is stripped so we get raw responses for easier introspection
const STRIPPED_REQUEST_HEADERS: &[&str] = &["host", "accept-encoding"];

// the following hop-by-hop headers should not be forwarded downstream
const STRIPPED_RESPONSE_HEADERS: &[&str] = &["transfer-encoding", "connection", "keep-alive"];

struct AppState {
    http_client: reqwest::Client,
}

struct ProviderContext {
    name: String,
    upstream: String,
}

pub async fn start(config: Config) -> anyhow::Result<()> {
    let state = Arc::new(AppState {
        http_client: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300)) // TODO: make configurable, and handle better with SSE?
            .build()?,
    });
    let app = build_router(&config, state);

    let listener = TcpListener::bind(&config.listen).await?;
    tracing::info!("proxy running at http://{}", listener.local_addr()?);
    axum::serve(listener, app).await?;

    Ok(())
}

fn build_router(config: &Config, state: Arc<AppState>) -> Router {
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
    Extension(provider): Extension<Arc<ProviderContext>>,
    client_req: Request<Body>,
) -> Response {
    let (parts, body) = client_req.into_parts();

    let path = parts.uri.path();
    let query = parts.uri.query();
    let upstream_url = match query {
        Some(q) => format!("{}{path}?{q}", provider.upstream),
        None => format!("{}{path}", provider.upstream),
    };

    let req_body_bytes = match axum::body::to_bytes(body, MAX_REQUEST_BODY_BYTES).await {
        Ok(bytes) => bytes,
        Err(err) => {
            tracing::error!(error = %err, "failed to read request body");
            return StatusCode::BAD_REQUEST.into_response();
        }
    };

    let mut upstream_req = state.http_client.request(parts.method, &upstream_url);
    for (key, value) in &parts.headers {
        if STRIPPED_REQUEST_HEADERS.contains(&key.as_str()) {
            continue;
        }
        upstream_req = upstream_req.header(key, value);
    }
    upstream_req = upstream_req.body(req_body_bytes);

    let upstream_response = match upstream_req.send().await {
        Ok(res) => res,
        Err(err) => {
            tracing::error!(
                provider = %provider.name,
                url = %upstream_url,
                error = %err,
                "upstream request failed",
            );
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };

    let up_status = upstream_response.status();
    let up_res_headers = upstream_response.headers().clone();
    let up_stream = upstream_response.bytes_stream();

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, String>>(64);

    let provider_name = provider.name.clone();
    tokio::spawn(async move {
        let mut stream = up_stream;

        tracing::warn!(provider = %provider_name, "we haven't implemented response processing yet!");

        // TODO: Extract the full "usage object" from the response.

        // 0. figure out if we are in streaming mode (SSE), content-type: text/event-stream?

        // 1. setup buffer here
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    // 2. append to buffer
                    if tx.send(Ok(bytes)).await.is_err() {
                        break;
                    }
                }
                Err(err) => {
                    tracing::error!(
                        provider = %provider_name,
                        error = %err,
                        "error reading upstream response chunk",
                    );
                    let _ = tx.send(Err(err.to_string())).await;
                    break;
                }
            }
        }
        // 3. if sse, parse SSE events from buffer and extract usage object from the final event
        //    if not, parse buffer as JSON and extract usage object
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
