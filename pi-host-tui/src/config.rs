//! TOML configuration for pi-host-tui.
//!
//! File discovery (project → home):
//! 1. `<cwd>/.pi-oxide/config.toml` (project)
//! 2. `~/.pi-oxide/config.toml` (global)
//!
//! Config only holds infrastructure settings (LLM provider, model, API key, base URL).
//! Per-session values (system prompt, session ID) are CLI or env var only.
//!
//! Precedence for each field: CLI flag > env var > config file > hardcoded default.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// LLM provider and model configuration.
///
/// This is the only section in config.toml. Everything else (system prompt,
/// session ID) is per-invocation and belongs on the CLI or in env vars.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct LlmConfig {
    pub model: String,
    pub provider: String,
    pub api_key: String,
    pub base_url: String,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            model: "claude-sonnet-5".into(),
            provider: "anthropic".into(),
            api_key: String::new(),
            base_url: String::new(),
        }
    }
}

/// Top-level configuration loaded from config.toml.
///
/// ponytail: single section for now. If we add more (compaction settings,
/// tool permissions), the top-level struct grows.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Config {
    pub llm: LlmConfig,
}

// ---------------------------------------------------------------------------
// File discovery
// ---------------------------------------------------------------------------

const CONFIG_DIR: &str = ".pi-oxide";
const CONFIG_FILE: &str = "config.toml";

/// Return the path to the project config file: `<cwd>/.pi-oxide/config.toml`.
pub fn project_config_path() -> PathBuf {
    std::env::current_dir()
        .ok()
        .map(|cwd| cwd.join(CONFIG_DIR).join(CONFIG_FILE))
        .unwrap_or_default()
}

/// Return the path to the global config file: `~/.pi-oxide/config.toml`.
pub fn global_config_path() -> PathBuf {
    std::env::home_dir()
        .unwrap_or_default()
        .join(CONFIG_DIR)
        .join(CONFIG_FILE)
}

/// Discover and load the config file.
///
/// Checks project config first, falls back to global.
/// Returns the defaults if no file exists.
/// Also returns the path of the file that was used, if any.
pub fn load() -> (Config, Option<PathBuf>) {
    let project = project_config_path();
    let global = global_config_path();

    if project.exists() {
        return match load_file(&project) {
            Ok(cfg) => (cfg, Some(project)),
            Err(e) => {
                eprintln!(
                    "Warning: failed to parse {} ({e}), falling back to global config",
                    project.display()
                );
                match load_file(&global) {
                    Ok(cfg) => (cfg, Some(global)),
                    Err(e2) => {
                        eprintln!(
                            "Warning: failed to load global config {}: {e2}",
                            global.display()
                        );
                        (Config::default(), None)
                    }
                }
            }
        };
    }

    if global.exists() {
        return match load_file(&global) {
            Ok(cfg) => (cfg, Some(global)),
            Err(e) => {
                eprintln!("Warning: failed to load {}: {e}", global.display());
                (Config::default(), None)
            }
        };
    }

    (Config::default(), None)
}

fn load_file(path: &Path) -> Result<Config, Box<dyn std::error::Error>> {
    let content = std::fs::read_to_string(path)?;
    let config: Config = toml::from_str(&content)?;
    Ok(config)
}

/// Write the config to the global config file `~/.pi-oxide/config.toml`.
pub fn write_global(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    let path = global_config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = toml::to_string_pretty(&config)?;
    std::fs::write(&path, content)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Resolution: merge config + env + defaults
// ---------------------------------------------------------------------------

/// Resolved configuration after merging all layers.
///
/// Only infrastructure settings. System prompt and session ID are resolved
/// separately via CLI/env and passed through by the caller.
#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    pub model: String,
    pub provider: String,
    pub api_key: String,
    pub base_url: String,
    /// Path of the config file that provided values, if any.
    pub config_path: Option<PathBuf>,
}

/// Resolve the final configuration.
///
/// Precedence per field: CLI override (if Some) > env var > config file > hardcoded default.
/// CLI overrides are passed as `Option` values; `None` means "not set by CLI".
pub fn resolve(
    cli_model: Option<&str>,
    cli_provider: Option<&str>,
    cli_api_key: Option<&str>,
    cli_base_url: Option<&str>,
) -> ResolvedConfig {
    let (config, config_path) = load();

    // Warn if API key is stored in the config file (env vars are preferred)
    if !config.llm.api_key.is_empty() {
        eprintln!(
            "Warning: API key is stored in config file. Use env vars (ANTHROPIC_API_KEY, OPENAI_API_KEY, etc.) instead for better security."
        );
    }

    let model = resolve_field(cli_model, "PI_MODEL", &config.llm.model, "claude-sonnet-5");
    let provider = resolve_field(
        cli_provider,
        "PI_PROVIDER",
        &config.llm.provider,
        "anthropic",
    );

    let api_key = resolve_api_key(&provider, cli_api_key, &config.llm.api_key);

    let base_url = resolve_field(
        cli_base_url,
        "PI_BASE_URL",
        &config.llm.base_url,
        &default_base_url(&provider),
    );

    ResolvedConfig {
        model,
        provider,
        api_key,
        base_url,
        config_path,
    }
}

/// Resolve a single config field: CLI > env var > config file > hardcoded default.
fn resolve_field(cli: Option<&str>, env_key: &str, config_value: &str, fallback: &str) -> String {
    cli.or(std::env::var(env_key).ok().as_deref())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            if config_value.is_empty() {
                None
            } else {
                Some(config_value.to_string())
            }
        })
        .unwrap_or_else(|| fallback.to_string())
}

/// Resolve API key with provider-specific env var priority.
fn resolve_api_key(provider: &str, cli_override: Option<&str>, config_value: &str) -> String {
    let env_key = match provider {
        "anthropic" | "anthropic-compat" => "ANTHROPIC_API_KEY",
        "openai" | "openai-compat" => "OPENAI_API_KEY",
        "deepseek" | "deepseek-anthropic" => "DEEPSEEK_API_KEY",
        _ => "PI_API_KEY",
    };

    cli_override
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| std::env::var(env_key).ok().filter(|s| !s.is_empty()))
        .or_else(|| std::env::var("PI_API_KEY").ok().filter(|s| !s.is_empty()))
        .or_else(|| {
            if config_value.is_empty() {
                None
            } else {
                Some(config_value.to_string())
            }
        })
        .unwrap_or_default()
}

/// Return the default base URL for a known provider.
fn default_base_url(provider: &str) -> String {
    match provider {
        "anthropic" | "anthropic-compat" => "https://api.anthropic.com".into(),
        "openai" | "openai-compat" => "https://api.openai.com".into(),
        "deepseek" => "https://api.deepseek.com".into(),
        "deepseek-anthropic" => "https://api.deepseek.com/anthropic".into(),
        _ => "https://api.anthropic.com".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.llm.model, "claude-sonnet-5");
        assert_eq!(config.llm.provider, "anthropic");
        assert!(config.llm.api_key.is_empty());
    }

    #[test]
    fn test_default_base_url() {
        assert_eq!(default_base_url("anthropic"), "https://api.anthropic.com");
        assert_eq!(default_base_url("openai"), "https://api.openai.com");
        assert_eq!(default_base_url("deepseek"), "https://api.deepseek.com");
        assert_eq!(
            default_base_url("deepseek-anthropic"),
            "https://api.deepseek.com/anthropic"
        );
    }

    #[test]
    fn test_toml_roundtrip() {
        let toml_str = r#"
[llm]
model = "claude-sonnet-5"
provider = "openai"
api-key = "sk-test"
base-url = "https://custom.api"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.llm.model, "claude-sonnet-5");
        assert_eq!(config.llm.provider, "openai");
        assert_eq!(config.llm.api_key, "sk-test");
        assert_eq!(config.llm.base_url, "https://custom.api");
    }

    #[test]
    fn test_toml_partial() {
        // Only llm.model set, rest should default
        let toml_str = r#"
[llm]
model = "gpt-4o"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.llm.model, "gpt-4o");
        assert_eq!(config.llm.provider, "anthropic"); // default
        assert!(config.llm.api_key.is_empty());
    }

    #[test]
    fn test_toml_kebab_case() {
        let toml_str = r#"
[llm]
api-key = "key"
base-url = "https://example.com"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.llm.api_key, "key");
        assert_eq!(config.llm.base_url, "https://example.com");
    }
}
