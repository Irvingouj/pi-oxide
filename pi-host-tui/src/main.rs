use clap::Parser;

use crate::host_state::HostState;

mod app;
mod extension;
mod host_state;
mod llm;
mod markdown;
mod session;
mod smoke_test;
mod tools;
mod tui;

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
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let api_key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_default();
    let base_url = cli
        .base_url
        .unwrap_or_else(|| "https://api.anthropic.com".into());

    let session_backend = session::FileSystemSessionBackend::new();
    let host_state = cli.session_id.as_ref().and_then(|id| {
        // Try new format first
        if let Some(data) = session_backend.load(id) {
            return Some(HostState::restore(data));
        }
        // Fall back to old SessionState format
        let path = session_backend.path_for(id);
        let data = std::fs::read_to_string(path).ok()?;
        let old: pi_core::SessionState = serde_json::from_str(&data).ok()?;
        Some(HostState::from_session_state(
            old,
            pi_core::ContextProjectionState::default(),
            std::collections::BTreeMap::new(),
            pi_core::ContextProjectionBudget::default(),
            cli.system.clone(),
        ))
    });
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
    );
    let result = app.run(&mut terminal, &session_backend);
    ratatui::restore();
    result
}
