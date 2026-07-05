//! Interactive onboarding wizard for first-time users.
//!
//! Runs before the main chat loop when no config file exists and no API key
//! is available from environment variables.
//!
//! Design decisions:
//! - No API key validation step — user verifies by sending a message.
//! - No "already onboarded" flag — presence of config file is the signal.

use std::io::{self, Write};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};

use crate::config::{self, Config, LlmConfig};

/// Result of a completed onboarding flow.
#[derive(Debug, Clone)]
pub struct OnboardingResult {
    pub provider: String,
    pub model: String,
    pub api_key: String,
    pub base_url: String,
}

/// Preset provider configurations.
struct ProviderPreset {
    label: &'static str,
    key: &'static str,
    default_model: &'static str,
    default_url: &'static str,
}

const PRESETS: &[ProviderPreset] = &[
    ProviderPreset {
        label: "Anthropic (Claude)",
        key: "anthropic",
        default_model: "claude-sonnet-5",
        default_url: "https://api.anthropic.com",
    },
    ProviderPreset {
        label: "OpenAI (GPT)",
        key: "openai",
        default_model: "gpt-5.5",
        default_url: "https://api.openai.com",
    },
    ProviderPreset {
        label: "DeepSeek",
        key: "deepseek",
        default_model: "deepseek-v4-flash",
        default_url: "https://api.deepseek.com",
    },
];

/// Run the onboarding wizard. Returns `None` if the user cancels.
pub fn run() -> Option<OnboardingResult> {
    println!("\n{}", "─".repeat(50));
    println!("  Welcome to pio — Terminal Coding Agent");
    println!("{}\n", "─".repeat(50));

    // Step 1: Provider selection
    let (provider, default_model, default_url) = select_provider()?;

    // Step 2: API key
    let api_key = read_api_key(&provider)?;

    // Step 3: Model (with default from provider)
    let model = read_model(&default_model)?;

    // Step 4: Base URL (with default from provider)
    let base_url = read_url(&default_url)?;

    // Persist config
    let config = Config {
        llm: LlmConfig {
            model: model.clone(),
            provider: provider.clone(),
            api_key: api_key.clone(),
            base_url: base_url.clone(),
        },
    };
    if let Err(e) = config::write_global(config) {
        eprintln!("Warning: could not write config file: {e}");
    } else {
        println!(
            "\nConfig saved to {}.",
            config::global_config_path().display()
        );
    }

    println!("\nAll set! Starting pio...\n");

    Some(OnboardingResult {
        provider,
        model,
        api_key,
        base_url,
    })
}

/// Step 1: Let the user pick a provider preset or go custom.
fn select_provider() -> Option<(String, String, String)> {
    loop {
        println!("Choose your provider:");
        for (i, p) in PRESETS.iter().enumerate() {
            println!("  [{}] {}", i + 1, p.label);
        }
        println!("  [{}] Custom", PRESETS.len() + 1);
        print!("\nSelection: ");
        io::stdout().flush().ok();

        let input = read_line()?;
        let choice: usize = input.parse().ok()?;

        if choice >= 1 && choice <= PRESETS.len() {
            let preset = &PRESETS[choice - 1];
            return Some((
                preset.key.to_string(),
                preset.default_model.to_string(),
                preset.default_url.to_string(),
            ));
        }
        if choice == PRESETS.len() + 1 {
            return select_custom_provider();
        }
        println!("Invalid selection. Please try again.");
    }
}

/// Custom provider: user enters provider family, default model, and default URL.
fn select_custom_provider() -> Option<(String, String, String)> {
    loop {
        print!("Provider family (openai / anthropic / openai-compat / anthropic-compat): ");
        io::stdout().flush().ok();
        let provider = read_line()?.trim().to_string();
        if provider.is_empty() {
            println!("Provider cannot be empty.");
            continue;
        }

        let default_url = match provider.as_str() {
            "openai" | "openai-compat" => "https://api.openai.com",
            "anthropic" | "anthropic-compat" => "https://api.anthropic.com",
            _ => "",
        };

        print!("Model ID (e.g. gpt-4o): ");
        io::stdout().flush().ok();
        let model = read_line()?.trim().to_string();
        if model.is_empty() {
            println!("Model ID cannot be empty.");
            continue;
        }

        return Some((provider, model, default_url.to_string()));
    }
}

/// Step 2: Read API key with masked input.
fn read_api_key(provider: &str) -> Option<String> {
    let hint = match provider {
        "anthropic" | "anthropic-compat" => "ANTHROPIC_API_KEY",
        "openai" | "openai-compat" => "OPENAI_API_KEY",
        "deepseek" => "DEEPSEEK_API_KEY",
        _ => "API_KEY",
    };

    loop {
        println!("\nEnter your {hint} (input will be hidden):");
        print!("  API key: ");
        io::stdout().flush().ok();

        let key = read_password()?;
        if key.is_empty() {
            println!("API key cannot be empty. Press Ctrl+C to cancel.");
            continue;
        }
        print!("  ********");
        io::stdout().flush().ok();
        println!();
        return Some(key);
    }
}

/// Step 3: Read model ID, pre-filled with the provider default.
fn read_model(default: &str) -> Option<String> {
    print!("\nModel ID [{default}]: ");
    io::stdout().flush().ok();

    let input = read_line()?;
    if input.trim().is_empty() {
        Some(default.to_string())
    } else {
        Some(input.trim().to_string())
    }
}

/// Step 4: Read base URL, pre-filled with the provider default.
fn read_url(default: &str) -> Option<String> {
    loop {
        let prompt = if default.is_empty() {
            "Base URL:".to_string()
        } else {
            format!("Base URL [{default}]:")
        };
        print!("\n{prompt} ");
        io::stdout().flush().ok();

        let input = read_line()?;
        if input.trim().is_empty() {
            if default.is_empty() {
                println!("Base URL cannot be empty.");
                continue;
            }
            return Some(default.to_string());
        }
        return Some(input.trim().to_string());
    }
}

// ---------------------------------------------------------------------------
// Terminal I/O helpers
// ---------------------------------------------------------------------------

/// Read a line from stdin.
fn read_line() -> Option<String> {
    let mut buf = String::new();
    io::stdin().read_line(&mut buf).ok()?;
    Some(buf.trim_end().to_string())
}

/// Read a password with hidden input via crossterm raw mode.
fn read_password() -> Option<String> {
    crossterm::execute!(io::stdout(), crossterm::terminal::EnterAlternateScreen).ok();
    crossterm::terminal::enable_raw_mode().ok();
    let key = read_password_raw();
    crossterm::terminal::disable_raw_mode().ok();
    crossterm::execute!(io::stdout(), crossterm::terminal::LeaveAlternateScreen).ok();
    key
}

fn read_password_raw() -> Option<String> {
    let mut password = String::new();

    loop {
        if !event::poll(std::time::Duration::from_millis(50)).ok()? {
            continue;
        }
        let ev = event::read().ok()?;
        if let Event::Key(KeyEvent {
            code,
            kind: KeyEventKind::Press,
            ..
        }) = ev
        {
            match code {
                KeyCode::Enter => {
                    println!();
                    return Some(password);
                }
                KeyCode::Char(c) => {
                    password.push(c);
                }
                KeyCode::Backspace => {
                    password.pop();
                }
                KeyCode::Esc => {
                    // Let Ctrl+C propagate naturally
                    return None;
                }
                _ => {}
            }
        }
    }
}
