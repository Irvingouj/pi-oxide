use clap::Parser;

mod app;
mod llm;
mod markdown;
mod tools;

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
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let api_key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_default();
    let base_url = cli
        .base_url
        .unwrap_or_else(|| "https://api.anthropic.com".into());

    let mut terminal = ratatui::init();
    let app = app::App::new(&cli.system, &cli.model, &api_key, &base_url);
    let result = app.run(&mut terminal);
    ratatui::restore();
    result
}
