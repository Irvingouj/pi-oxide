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

/// Return the user's home directory.
///
/// Checks `$HOME`, then `$USERPROFILE` (Windows), then falls back to the
/// current directory.
pub(crate) fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_default())
}

/// Return the path to the project config file: `<cwd>/.pi-oxide/config.toml`.
pub fn project_config_path() -> PathBuf {
    std::env::current_dir()
        .ok()
        .map(|cwd| cwd.join(CONFIG_DIR).join(CONFIG_FILE))
        .unwrap_or_default()
}

/// Return the path to the global config file: `~/.pi-oxide/config.toml`.
pub fn global_config_path() -> PathBuf {
    home_dir()
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

/// Detect a provider from whichever API key environment variable is set.
///
/// Used as a fallback when no explicit provider is configured but an API key
/// is available — prevents the default "anthropic" provider from being chosen
/// when the user only has, say, DEEPSEEK_API_KEY set.
fn detect_provider_from_env() -> Option<String> {
    if std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .is_some_and(|v| !v.is_empty())
    {
        return Some("anthropic".into());
    }
    if std::env::var("OPENAI_API_KEY")
        .ok()
        .is_some_and(|v| !v.is_empty())
    {
        return Some("openai".into());
    }
    if std::env::var("DEEPSEEK_API_KEY")
        .ok()
        .is_some_and(|v| !v.is_empty())
    {
        return Some("deepseek".into());
    }
    None
}

/// Resolve the final configuration.
///
/// Precedence per field: CLI override (if Some) > env var > config file > auto-detect > hardcoded default.
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

    // Resolve provider: CLI > env > config file > auto-detect from API key env > hardcoded default.
    let provider = if config_path.is_none()
        && cli_provider.is_none()
        && std::env::var("PI_PROVIDER")
            .ok()
            .filter(|s| !s.is_empty())
            .is_none()
    {
        // No explicit provider anywhere — try auto-detecting from API key env vars
        detect_provider_from_env()
            .unwrap_or_else(|| resolve_field(None, "PI_PROVIDER", "", "anthropic"))
    } else {
        resolve_field(
            cli_provider,
            "PI_PROVIDER",
            &config.llm.provider,
            "anthropic",
        )
    };

    // Resolve model: CLI > env > config file (only if from real file) > provider-aware default.
    let model_from_config = if config_path.is_some() {
        &config.llm.model
    } else {
        // No config file — skip the default model so we fall through to provider-aware default
        ""
    };
    let model = resolve_field(
        cli_model,
        "PI_MODEL",
        model_from_config,
        resolve_default_model(&provider),
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

/// Return the default model ID for a known provider.
pub fn resolve_default_model(provider: &str) -> &'static str {
    match provider {
        "anthropic" | "anthropic-compat" => "claude-sonnet-5",
        "openai" | "openai-compat" => "gpt-5.5",
        "deepseek" | "deepseek-anthropic" => "deepseek-v4-pro",
        _ => "claude-sonnet-5",
    }
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

    // -----------------------------------------------------------------------
    // Auto-detect provider from API key env vars
    // -----------------------------------------------------------------------

    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::sync::Mutex;

    static RESOLVE_LOCK: Mutex<()> = Mutex::new(());

    const ISOLATE_KEYS: &[&str] = &[
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "DEEPSEEK_API_KEY",
        "PI_API_KEY",
        "PI_MODEL",
        "PI_PROVIDER",
        "PI_BASE_URL",
    ];

    struct ResolveGuard {
        saved_env: HashMap<String, Option<OsString>>,
        saved_dir: Option<std::path::PathBuf>,
        temp_dir: Option<PathBuf>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl Drop for ResolveGuard {
        fn drop(&mut self) {
            // Restore env vars
            for (key, value) in &self.saved_env {
                match value {
                    Some(v) => std::env::set_var(key, v),
                    None => std::env::remove_var(key),
                }
            }
            // Restore working directory
            if let Some(ref dir) = self.saved_dir {
                let _ = std::env::set_current_dir(dir);
            }
            // Clean up temp directory
            if let Some(ref dir) = self.temp_dir {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }

    fn isolate_env() -> ResolveGuard {
        let _lock = RESOLVE_LOCK.lock().expect("RESOLVE_LOCK");
        let dir = std::env::temp_dir().join(format!(
            "pi-resolve-test-{}-{}",
            std::process::id(),
            std::time::Instant::now().elapsed().as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create temp");

        // Save and clear all relevant env vars
        let mut saved_env = HashMap::new();
        for &key in ISOLATE_KEYS {
            saved_env.insert(key.to_string(), std::env::var_os(key));
            std::env::remove_var(key);
        }
        // Save HOME
        saved_env.insert("HOME".to_string(), std::env::var_os("HOME"));
        std::env::set_var("HOME", &dir);

        ResolveGuard {
            saved_env,
            saved_dir: None,
            temp_dir: Some(dir),
            _lock,
        }
    }

    impl ResolveGuard {
        fn temp_dir(&self) -> &PathBuf {
            self.temp_dir.as_ref().expect("temp_dir already cleaned up")
        }
    }

    #[test]
    fn test_detect_provider_from_env_deepseek() {
        let _guard = isolate_env();
        std::env::set_var("DEEPSEEK_API_KEY", "sk-test");
        assert_eq!(detect_provider_from_env(), Some("deepseek".into()));
    }

    #[test]
    fn test_detect_provider_from_env_anthropic() {
        let _guard = isolate_env();
        std::env::set_var("ANTHROPIC_API_KEY", "sk-test");
        assert_eq!(detect_provider_from_env(), Some("anthropic".into()));
    }

    #[test]
    fn test_detect_provider_from_env_openai() {
        let _guard = isolate_env();
        std::env::set_var("OPENAI_API_KEY", "sk-test");
        assert_eq!(detect_provider_from_env(), Some("openai".into()));
    }

    #[test]
    fn test_detect_provider_from_env_none() {
        let _guard = isolate_env();
        assert_eq!(detect_provider_from_env(), None);
    }

    #[test]
    fn test_resolve_auto_detects_deepseek_when_no_config() {
        let _guard = isolate_env();
        std::env::set_var("DEEPSEEK_API_KEY", "sk-deepseek-test");

        let resolved = resolve(None, None, None, None);
        assert_eq!(resolved.provider, "deepseek");
        assert_eq!(resolved.api_key, "sk-deepseek-test");
        assert_eq!(resolved.base_url, "https://api.deepseek.com");
    }

    #[test]
    fn test_resolve_explicit_provider_overrides_auto_detect() {
        let _guard = isolate_env();
        std::env::set_var("DEEPSEEK_API_KEY", "sk-deepseek");
        std::env::set_var("ANTHROPIC_API_KEY", "sk-anthropic");
        std::env::set_var("PI_PROVIDER", "anthropic");

        let resolved = resolve(None, None, None, None);
        assert_eq!(resolved.provider, "anthropic");
        assert_eq!(resolved.api_key, "sk-anthropic");
    }

    // -----------------------------------------------------------------------
    // Provider-aware default model
    // -----------------------------------------------------------------------

    #[test]
    fn test_default_model_for_anthropic() {
        assert_eq!(resolve_default_model("anthropic"), "claude-sonnet-5");
        assert_eq!(resolve_default_model("anthropic-compat"), "claude-sonnet-5");
    }

    #[test]
    fn test_default_model_for_openai() {
        assert_eq!(resolve_default_model("openai"), "gpt-5.5");
        assert_eq!(resolve_default_model("openai-compat"), "gpt-5.5");
    }

    #[test]
    fn test_default_model_for_deepseek() {
        assert_eq!(resolve_default_model("deepseek"), "deepseek-v4-pro");
        assert_eq!(
            resolve_default_model("deepseek-anthropic"),
            "deepseek-v4-pro"
        );
    }

    #[test]
    fn test_default_model_fallback() {
        assert_eq!(resolve_default_model("unknown"), "claude-sonnet-5");
    }

    #[test]
    fn test_resolve_auto_detects_deepseek_model() {
        let _guard = isolate_env();
        std::env::set_var("DEEPSEEK_API_KEY", "sk-deepseek-test");

        let resolved = resolve(None, None, None, None);
        assert_eq!(resolved.provider, "deepseek");
        assert_eq!(resolved.model, "deepseek-v4-pro");
    }

    #[test]
    fn test_resolve_auto_detects_openai_model() {
        let _guard = isolate_env();
        std::env::set_var("OPENAI_API_KEY", "sk-openai-test");

        let resolved = resolve(None, None, None, None);
        assert_eq!(resolved.provider, "openai");
        assert_eq!(resolved.model, "gpt-5.5");
    }

    #[test]
    fn test_resolve_auto_detects_anthropic_model() {
        let _guard = isolate_env();
        std::env::set_var("ANTHROPIC_API_KEY", "sk-anthropic-test");

        let resolved = resolve(None, None, None, None);
        assert_eq!(resolved.provider, "anthropic");
        assert_eq!(resolved.model, "claude-sonnet-5");
    }

    #[test]
    fn test_cli_model_overrides_auto_detected_default() {
        let _guard = isolate_env();
        std::env::set_var("DEEPSEEK_API_KEY", "sk-deepseek-test");

        let resolved = resolve(Some("deepseek-reasoner"), None, None, None);
        assert_eq!(resolved.provider, "deepseek");
        assert_eq!(resolved.model, "deepseek-reasoner");
    }

    #[test]
    fn test_config_file_model_overrides_provider_default() {
        let mut guard = isolate_env();
        std::env::set_var("DEEPSEEK_API_KEY", "sk-deepseek-test");
        let dir = guard.temp_dir().clone();

        // Write a config file with a custom model
        let config_path = dir.join(".pi-oxide");
        std::fs::create_dir_all(&config_path).unwrap();
        std::fs::write(
            config_path.join("config.toml"),
            r#"
[llm]
model = "deepseek-reasoner"
provider = "deepseek"
"#,
        )
        .unwrap();

        // Change to the test directory so project config is found
        guard.saved_dir = Some(std::env::current_dir().expect("current dir"));
        std::env::set_current_dir(&dir).expect("set_current_dir");

        let resolved = resolve(None, None, None, None);
        assert_eq!(resolved.provider, "deepseek");
        assert_eq!(resolved.model, "deepseek-reasoner"); // from config file, not default
    }

    // -----------------------------------------------------------------------
    // Edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn test_empty_api_key_treated_as_unset() {
        let _guard = isolate_env();
        std::env::set_var("ANTHROPIC_API_KEY", "");
        assert_eq!(detect_provider_from_env(), None);
    }

    #[test]
    fn test_priority_anthropic_wins_over_deepseek() {
        let _guard = isolate_env();
        std::env::set_var("ANTHROPIC_API_KEY", "sk-ant");
        std::env::set_var("DEEPSEEK_API_KEY", "sk-ds");
        assert_eq!(detect_provider_from_env(), Some("anthropic".into()));
    }

    #[test]
    fn test_priority_openai_wins_over_deepseek() {
        let _guard = isolate_env();
        std::env::set_var("OPENAI_API_KEY", "sk-oai");
        std::env::set_var("DEEPSEEK_API_KEY", "sk-ds");
        assert_eq!(detect_provider_from_env(), Some("openai".into()));
    }

    #[test]
    fn test_pi_api_key_fallback() {
        let _guard = isolate_env();
        std::env::set_var("PI_API_KEY", "sk-generic");
        std::env::set_var("PI_PROVIDER", "openai");

        let resolved = resolve(None, None, None, None);
        assert_eq!(resolved.provider, "openai");
        assert_eq!(resolved.api_key, "sk-generic");
    }
}
