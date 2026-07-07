use clap::Parser;

use crate::host_state::HostState;

mod agent_host;
mod app;
mod commands;
mod config;
#[cfg(all(test, unix))]
mod e2e_tests;
mod editor;
mod extension;
mod host_state;
#[cfg(test)]
mod input_tests;
mod llm;
#[cfg(any(feature = "record", feature = "replay"))]
mod llm_cassette;
#[cfg(feature = "record")]
mod llm_record;
#[cfg(feature = "replay")]
mod llm_replay;
mod markdown;
mod model_picker;
mod onboarding;
#[cfg(test)]
mod onboarding_test;
#[cfg(all(test, unix))]
mod record_replay_test;
#[cfg(all(test, unix))]
mod replay_e2e_test;
mod scroll;
mod session;
mod session_log;
mod smoke_test;
mod theme;
mod tools;
#[cfg(test)]
mod tools_test;
mod tui;

#[cfg(test)]
#[cfg(feature = "replay")]
mod replay_e2e;

#[derive(Parser)]
#[command(name = "pio", about = "Terminal coding agent")]
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
    /// Skip the onboarding wizard
    #[arg(long)]
    skip_onboarding: bool,
}

/// Check whether onboarding should run: no config file found and no API key from env.
fn should_run_onboarding(cli: &Cli) -> bool {
    // If user provided any infra override, they know what they're doing
    if cli.model.is_some()
        || cli.provider.is_some()
        || cli.api_key.is_some()
        || cli.base_url.is_some()
    {
        return false;
    }
    // If a config file exists, skip onboarding
    if config::project_config_path().exists() || config::global_config_path().exists() {
        return false;
    }
    // If any API key env var is set, skip onboarding
    let api_key_envs = [
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "DEEPSEEK_API_KEY",
        "PI_API_KEY",
    ];
    for key in &api_key_envs {
        if std::env::var(key).ok().is_some_and(|v| !v.is_empty()) {
            return false;
        }
    }
    true
}

fn main() -> Result<(), app::TuiError> {
    let log_dir = config::home_dir().join(".pi-oxide").join("logs");
    std::fs::create_dir_all(&log_dir).ok();
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_dir.join("pio.log"))
        .expect("failed to open log file");

    tracing_subscriber::fmt()
        .with_writer(log_file)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let cli = Cli::parse();

    // Run onboarding if no config exists and no API key is available
    let resolved = if !cli.skip_onboarding && should_run_onboarding(&cli) {
        if let Some(onboard) = onboarding::run() {
            // Re-resolve with onboarding results as CLI overrides
            config::resolve(
                Some(&onboard.model),
                Some(&onboard.provider),
                Some(&onboard.api_key),
                Some(&onboard.base_url),
            )
        } else {
            // User cancelled onboarding; fall back to defaults
            config::resolve(
                cli.model.as_deref(),
                cli.provider.as_deref(),
                cli.api_key.as_deref(),
                cli.base_url.as_deref(),
            )
        }
    } else {
        // Merge infrastructure config: CLI > env > config file > hardcoded defaults
        config::resolve(
            cli.model.as_deref(),
            cli.provider.as_deref(),
            cli.api_key.as_deref(),
            cli.base_url.as_deref(),
        )
    };

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
    // Set cursor to steady block — matches the ▌ accent bar visually.
    print!("\x1b[2 q"); // ANSI: steady block cursor
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
    // Reset cursor to default on exit
    print!("\x1b[0 q");
    result
}
