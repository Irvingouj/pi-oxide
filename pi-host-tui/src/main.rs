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
    /// Provider family: anthropic, openai, deepseek, deepseek-anthropic, openai-compat, anthropic-compat
    #[arg(long, env = "PI_PROVIDER", default_value = "anthropic")]
    provider: String,
    /// API key (fallback; provider-specific env vars take precedence)
    #[arg(long, env = "PI_API_KEY")]
    api_key: Option<String>,
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

    // Resolve wire format from provider
    let wire_format = match cli.provider.as_str() {
        "anthropic" | "anthropic-compat" | "deepseek-anthropic" => {
            crate::llm::WireFormat::Anthropic
        }
        "openai" | "openai-compat" | "deepseek" => crate::llm::WireFormat::OpenAI,
        other => {
            eprintln!("Unknown provider: {other}. Supported: anthropic, openai, deepseek, deepseek-anthropic, openai-compat, anthropic-compat");
            std::process::exit(1);
        }
    };

    // Resolve API key: provider-specific env var first, then --api-key / PI_API_KEY
    let api_key = match cli.provider.as_str() {
        "anthropic" | "anthropic-compat" => std::env::var("ANTHROPIC_API_KEY")
            .ok()
            .or_else(|| cli.api_key.clone())
            .unwrap_or_default(),
        "openai" | "openai-compat" => std::env::var("OPENAI_API_KEY")
            .ok()
            .or_else(|| cli.api_key.clone())
            .unwrap_or_default(),
        "deepseek" | "deepseek-anthropic" => std::env::var("DEEPSEEK_API_KEY")
            .ok()
            .or_else(|| cli.api_key.clone())
            .unwrap_or_default(),
        _ => cli.api_key.clone().unwrap_or_default(),
    };

    // Resolve base URL from provider
    let base_url = cli
        .base_url
        .clone()
        .unwrap_or_else(|| match cli.provider.as_str() {
            "anthropic" => "https://api.anthropic.com".into(),
            "openai" => "https://api.openai.com".into(),
            "deepseek" => "https://api.deepseek.com".into(),
            "deepseek-anthropic" => "https://api.deepseek.com/anthropic".into(),
            "openai-compat" | "anthropic-compat" => {
                eprintln!("{} requires --base-url", cli.provider);
                std::process::exit(1);
            }
            _ => "https://api.anthropic.com".into(),
        });

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
        wire_format,
        &cli.provider,
        #[cfg(feature = "record")]
        cli.record_to,
        #[cfg(feature = "replay")]
        cli.replay_from,
    )?;
    let result = app.run(&mut terminal, &session_backend);
    ratatui::restore();
    result
}
