//! `pi-record-server` — HTTP recording/replay proxy for LLM API streams.
//!
//! ## Usage
//!
//! ### Record mode
//! ```sh
//! pi-record-server record \
//!   --target https://api.deepseek.com \
//!   --port 9999 \
//!   --output tests/fixtures/deepseek_chat.cassette.json
//! ```
//! Then run the TUI pointing at the proxy:
//! ```sh
//! PI_BASE_URL=http://localhost:9999 PI_PROVIDER=deepseek PI_API_KEY=sk-xxx pio
//! ```
//! Press Ctrl+C to stop recording and save the cassette.
//!
//! ### Replay mode
//! ```sh
//! pi-record-server replay \
//!   --cassette tests/fixtures/deepseek_chat.cassette.json \
//!   --port 9999
//! ```
//! Then run the TUI against the replay server (no API key needed):
//! ```sh
//! PI_BASE_URL=http://localhost:9999 PI_PROVIDER=deepseek PI_API_KEY=any pio
//! ```

mod cassette;
mod record;
mod replay;

use axum::body::Body;
use axum::http::StatusCode;
use axum::response::Response;

use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "pi-record-server", about = "LLM API recording/replay proxy")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Record mode: proxy requests to a real API and capture SSE streams.
    Record {
        /// Upstream API base URL, e.g. https://api.deepseek.com
        #[arg(long, default_value = "https://api.deepseek.com")]
        target: String,

        /// Port to listen on.
        #[arg(long, default_value = "9999")]
        port: u16,

        /// Output cassette file path.
        #[arg(long, default_value = "cassette.json")]
        output: String,
    },

    /// Replay mode: serve recorded SSE streams from a cassette file.
    Replay {
        /// Cassette file to replay from.
        #[arg(long, default_value = "cassette.json")]
        cassette: String,

        /// Port to listen on.
        #[arg(long, default_value = "9999")]
        port: u16,

        /// Delay between SSE chunks in ms (0 = send all at once).
        #[arg(long, default_value = "0")]
        delay_ms: u64,
    },
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("pi_record_server=info")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Command::Record {
            target,
            port,
            output,
        } => {
            record::run(target, port, output).await;
        }
        Command::Replay {
            cassette,
            port,
            delay_ms,
        } => {
            replay::run(cassette, port, delay_ms).await;
        }
    }
}

/// Shared model discovery endpoint used by both record and replay servers.
pub(crate) async fn models_handler() -> Response {
    let body = serde_json::json!({
        "object": "list",
        "data": [
            {"id": "deepseek-chat", "object": "model"},
            {"id": "deepseek-reasoner", "object": "model"}
        ]
    });
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap()
}
