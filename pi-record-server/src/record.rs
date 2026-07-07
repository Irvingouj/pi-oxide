//! Record mode — proxies requests to a real LLM API and captures raw SSE bytes.
//!
//! Starts an axum HTTP server on the given port. Every POST /v1/chat/completions
//! is forwarded to the upstream target. The SSE response is streamed back to the
//! client while simultaneously captured. On shutdown (SIGINT), the captured
//! cassette is written to disk.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use futures::StreamExt;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use crate::cassette::{Cassette, CassetteEntry, RecordedRequest, RecordedResponse};
use crate::models_handler;

/// Shared state for the record server.
pub struct RecordState {
    /// Upstream API base URL, e.g. "https://api.deepseek.com"
    pub target: String,
    /// HTTP client (reused for connection pooling)
    pub client: reqwest::Client,
    /// Captured entries, appended during proxying
    pub entries: Mutex<Vec<CassetteEntry>>,
}

/// Start the record server. Blocks until SIGINT, then saves the cassette.
pub async fn run(target: String, port: u16, output_path: String) {
    let state = Arc::new(RecordState {
        target: target.trim_end_matches('/').to_string(),
        client: reqwest::Client::new(),
        entries: Mutex::new(Vec::new()),
    });

    let app = Router::new()
        .route("/v1/chat/completions", post(proxy_handler))
        .route("/v1/models", get(models_handler))
        .with_state(state.clone());

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();

    info!(
        target = %state.target,
        port = port,
        output = %output_path,
        "record server listening — point PI_BASE_URL=http://localhost:{port}"
    );

    // Graceful shutdown on SIGINT
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .unwrap();

    // Save cassette
    let entries = state.entries.lock().await.clone();
    let cassette = Cassette {
        version: 2,
        target: state.target.clone(),
        entries,
    };
    let json = serde_json::to_string_pretty(&cassette).unwrap();
    std::fs::write(&output_path, &json).unwrap();
    info!(path = %output_path, entries = cassette.entries.len(), "cassette saved");
}

/// Proxy handler: forward request to upstream, stream SSE response to client, capture bytes.
async fn proxy_handler(
    State(state): State<Arc<RecordState>>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let url = format!("{}/v1/chat/completions", state.target);

    info!(url = %url, body_len = body.len(), "proxying request");

    // Forward to upstream with the same headers
    let mut req = state.client.post(&url).body(body.clone());

    // Copy relevant headers from incoming request
    for (key, value) in headers.iter() {
        let key_str = key.as_str().to_lowercase();
        // Skip host and connection headers; pass through auth and content-type
        if matches!(key_str.as_str(), "host" | "connection" | "content-length") {
            continue;
        }
        req = req.header(key.as_str(), value.as_bytes());
    }

    let upstream_resp = match req.send().await {
        Ok(resp) => resp,
        Err(e) => {
            error!(error = %e, "upstream request failed");
            return Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(Body::from(format!("Upstream error: {e}")))
                .unwrap();
        }
    };

    let upstream_status = upstream_resp.status();

    if !upstream_status.is_success() {
        let text = upstream_resp.text().await.unwrap_or_default();
        error!(status = %upstream_status, body = %text, "upstream error response");
        return Response::builder()
            .status(upstream_status)
            .body(Body::from(text))
            .unwrap();
    }

    // Create a channel to tee the response stream
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<bytes::Bytes, String>>(64);
    let state_clone = state.clone();
    let request_body = body;

    // Spawn a task that reads from upstream and writes to the channel + buffer
    tokio::spawn(async move {
        let mut stream = upstream_resp.bytes_stream();
        let mut all_bytes: Vec<u8> = Vec::new();

        while let Some(chunk_result) = stream.next().await {
            match chunk_result {
                Ok(chunk) => {
                    all_bytes.extend_from_slice(&chunk);
                    if tx.send(Ok(chunk)).await.is_err() {
                        // Client disconnected — still save what we have
                        break;
                    }
                }
                Err(e) => {
                    warn!(error = %e, "upstream stream error");
                    let _ = tx.send(Err("upstream stream error".to_string())).await;
                    break;
                }
            }
        }

        // Record the entry
        let entry = CassetteEntry {
            request: RecordedRequest {
                method: "POST".to_string(),
                url: "/v1/chat/completions".to_string(),
                body_json: request_body,
            },
            response: RecordedResponse {
                status: upstream_status.as_u16(),
                body_base64: base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    &all_bytes,
                ),
            },
        };
        state_clone.entries.lock().await.push(entry);
        info!(response_bytes = all_bytes.len(), "entry recorded");
    });

    // Return streaming response to the client
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/event-stream")
        .header("cache-control", "no-cache")
        .header("connection", "keep-alive")
        .body(Body::from_stream(stream))
        .unwrap()
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install SIGINT handler");
    };

    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    info!("shutting down record server...");
}
