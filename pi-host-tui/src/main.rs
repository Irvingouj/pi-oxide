use clap::Parser;

use crate::host_state::HostState;

mod app;
mod config;
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
#[cfg(test)]
mod tools_test;
mod tui;

#[cfg(test)]
#[cfg(feature = "replay")]
mod replay_e2e;

#[derive(Parser)]
#[command(name = "pi", about = "Terminal coding agent")]
struct Cli {
    /// Model ID (overrides config)
    #[arg(long, env = "PI_MODEL")]
    model: Option<String>,

    /// API base URL (overrides config)
    #[arg(long, env = "PI_BASE_URL")]
    base_url: Option<String>,

    /// Provider family: anthropic, openai, deepseek, deepseek-anthropic, openai-compat, anthropic-compat
    #[arg(long, env = "PI_PROVIDER")]
    provider: Option<String>,

    /// API key (overrides config and env vars)
    #[arg(long, env = "PI_API_KEY")]
    api_key: Option<String>,

    /// System prompt (per-invocation, not persisted in config)
    #[arg(long)]
    system: Option<String>,

    /// Session ID for persistent conversation history (per-invocation, not persisted in config)
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

    // Merge infrastructure config: CLI > env > config file > hardcoded defaults
    let resolved = config::resolve(
        cli.model.as_deref(),
        cli.provider.as_deref(),
        cli.api_key.as_deref(),
        cli.base_url.as_deref(),
    );

    // Per-invocation values: CLI > env > hardcoded default (never from config)
    let system_prompt = cli
        .system
        .clone()
        .unwrap_or_else(|| "You are a helpful coding assistant.".into());

    // Validate provider and resolve wire format
    let wire_format = match resolved.provider.as_str() {
        "anthropic" | "anthropic-compat" | "deepseek-anthropic" => {
            crate::llm::WireFormat::Anthropic
        }
        "openai" | "openai-compat" | "deepseek" => crate::llm::WireFormat::OpenAI,
        other => {
            eprintln!("Unknown provider: {other}. Supported: anthropic, openai, deepseek, deepseek-anthropic, openai-compat, anthropic-compat");
            eprintln!("Check .pi-oxide/config.toml, ~/.pi-oxide/config.toml, or --provider.");
            std::process::exit(1);
        }
    };

    // Load session if requested
    let session_backend = session::FileSystemSessionBackend::new();
    let mut host_state = None;

    if let Some(ref id) = cli.session_id {
        if let Some(data) = session_backend.load(id) {
            host_state = Some(HostState::restore(data.clone()));
        }
    }

    let cwd = std::env::current_dir()?;

    let mut terminal = ratatui::init();
    let app = app::App::new(
        &system_prompt,
        &resolved.model,
        &resolved.api_key,
        &resolved.base_url,
        cli.session_id,
        host_state,
        &cwd,
        wire_format,
        &resolved.provider,
        #[cfg(feature = "record")]
        cli.record_to,
        #[cfg(feature = "replay")]
        cli.replay_from,
        resolved.clone(),
    )?;
    let result = app.run(&mut terminal, &session_backend);
    ratatui::restore();
    result
}
