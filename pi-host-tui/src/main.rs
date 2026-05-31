use clap::Parser;

use crate::host_state::HostState;

mod app;
mod extension;
mod host_state;
mod llm;
#[cfg(any(feature = "record", feature = "replay"))]
mod llm_cassette;
#[cfg(feature = "record")]
mod llm_record;
#[cfg(feature = "replay")]
mod llm_replay;
mod markdown;
mod session;
mod smoke_test;
mod tools;
mod tui;

#[cfg(test)]
#[cfg(feature = "replay")]
mod replay_e2e;

#[derive(Parser)]
#[command(name = "pi", about = "Terminal coding agent")]
struct Cli {
    /// Model ID
    #[arg(long, env = "PI_MODEL", default_value = "claude-sonnet-4-20250514")]
    model: String,
    /// API base URL (supports Anthropic-compatible endpoints)
    #[arg(long, env = "PI_BASE_URL")]
    base_url: Option<String>,
    /// System prompt
    #[arg(long, default_value = "You are a helpful coding assistant.")]
    system: String,
    /// Session ID for persistent conversation history
    #[arg(short, long, env = "PI_SESSION_ID")]
    session_id: Option<String>,
    /// Record LLM calls to a cassette file (requires `record` feature)
    #[cfg(feature = "record")]
    #[arg(long)]
    record_to: Option<std::path::PathBuf>,
    /// Replay LLM calls from a cassette file (requires `replay` feature)
    #[cfg(feature = "replay")]
    #[arg(long)]
    replay_from: Option<std::path::PathBuf>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let api_key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_default();
    let base_url = cli
        .base_url
        .unwrap_or_else(|| "https://api.anthropic.com".into());

    let session_backend = session::FileSystemSessionBackend::new();
    let mut host_state = None;

    if let Some(id) = cli.session_id.as_ref() {
        if let Some(data) = session_backend.load(id) {
            host_state = Some(HostState::restore(data.clone()));
        }
    }

    let cwd = std::env::current_dir()?;

    let mut terminal = ratatui::init();
    let app = app::App::new(
        &cli.system,
        &cli.model,
        &api_key,
        &base_url,
        cli.session_id,
        host_state,
        &cwd,
        #[cfg(feature = "record")]
        cli.record_to,
        #[cfg(feature = "replay")]
        cli.replay_from,
    )?;
    let result = app.run(&mut terminal, &session_backend);
    ratatui::restore();
    result
}
