//! Replay mode — serves recorded cassettes as an HTTP API.
//!
//! Loads a cassette file and serves each recorded entry sequentially.
//! The TUI points `PI_BASE_URL` at this server and receives byte-for-byte
//! replicas of the original SSE streams.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use futures::stream::{self, StreamExt};
use tracing::{info, warn};

use crate::cassette::Cassette;
use crate::models_handler;

pub struct ReplayState {
    /// Loaded cassette entries (responses only, in order)
    responses: Vec<Vec<u8>>,
    /// Current entry index (atomic for concurrent safety)
    index: AtomicUsize,
    /// Delay between chunks in ms (0 = send all at once)
    chunk_delay_ms: u64,
}

/// Start the replay server. Blocks until SIGINT.
pub async fn run(cassette_path: String, port: u16, chunk_delay_ms: u64) {
    let json = std::fs::read_to_string(&cassette_path).unwrap_or_else(|e| {
        panic!("failed to read cassette file {cassette_path}: {e}");
    });
    let cassette: Cassette = serde_json::from_str(&json).unwrap_or_else(|e| {
        panic!("failed to parse cassette {cassette_path}: {e}");
    });

    let responses: Vec<Vec<u8>> = cassette
        .entries
        .iter()
        .map(|entry| {
            base64::Engine::decode(
                &base64::engine::general_purpose::STANDARD,
                &entry.response.body_base64,
            )
            .unwrap_or_else(|e| {
                panic!("failed to decode base64 response body: {e}");
            })
        })
        .collect();

    info!(
        entries = responses.len(),
        target = %cassette.target,
        "loaded cassette for replay"
    );

    let state = Arc::new(ReplayState {
        responses,
        index: AtomicUsize::new(0),
        chunk_delay_ms,
    });

    let app = Router::new()
        .route("/v1/chat/completions", post(replay_handler))
        .route("/v1/models", get(models_handler))
        .with_state(state.clone());

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();

    info!(
        port = port,
        "replay server listening — point PI_BASE_URL=http://localhost:{port}"
    );

    let shutdown = async {
        let ctrl_c = tokio::signal::ctrl_c();
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
        info!("shutting down replay server...");
    };

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
        .unwrap();
}

/// Replay handler: return the next recorded SSE response.
async fn replay_handler(State(state): State<Arc<ReplayState>>) -> Response {
    let idx = state.index.fetch_add(1, Ordering::SeqCst);

    let bytes = match state.responses.get(idx) {
        Some(b) => b.clone(),
        None => {
            warn!(call = idx, "cassette exhausted");
            return Response::builder()
                .status(StatusCode::SERVICE_UNAVAILABLE)
                .body(Body::from("cassette exhausted"))
                .unwrap();
        }
    };

    info!(call = idx, response_bytes = bytes.len(), "replaying entry");

    let delay_ms = state.chunk_delay_ms;

    // Stream the bytes in chunks to simulate real SSE streaming
    let stream = async_stream(bytes, delay_ms);
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/event-stream")
        .header("cache-control", "no-cache")
        .header("connection", "keep-alive")
        .body(Body::from_stream(stream))
        .unwrap()
}

/// Split bytes into chunks and stream them with optional delay.
fn async_stream(
    data: Vec<u8>,
    delay_ms: u64,
) -> impl futures::Stream<Item = Result<bytes::Bytes, String>> {
    // If no delay, send as a single chunk.
    // With delay, split into 256-byte chunks to simulate network packets.
    let chunks: Vec<Vec<u8>> = if delay_ms == 0 {
        vec![data]
    } else {
        data.chunks(256).map(|c| c.to_vec()).collect()
    };
    let delay = Duration::from_millis(delay_ms);
    stream::iter(chunks).then(move |chunk| {
        let delay = delay;
        async move {
            if delay_ms > 0 {
                tokio::time::sleep(delay).await;
            }
            Ok(bytes::Bytes::from(chunk))
        }
    })
}

