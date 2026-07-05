//! Unit / integration tests for the onboarding flow.
//!
//! Tests config write/read roundtrips, provider preset defaults,
//! and the resolution pipeline — all without spawning a PTY.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::config::{self, Config, LlmConfig};
use crate::onboarding::OnboardingResult;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Global mutex to serialize tests that manipulate HOME.
/// cargo test runs tests in parallel by default, and HOME is process-global.
static HOME_LOCK: Mutex<()> = Mutex::new(());

/// Create a unique temp directory and return it. The caller is responsible
/// for cleaning it up (via the returned guard or manually).
fn temp_home() -> (PathBuf, TempDirGuard) {
    let dir = std::env::temp_dir().join(format!("pi-test-{}-{}", std::process::id(), unique_id()));
    std::fs::create_dir_all(&dir).expect("create temp home");
    (dir.clone(), TempDirGuard(dir))
}

struct TempDirGuard(PathBuf);
impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn unique_id() -> u64 {
    std::time::Instant::now().elapsed().as_nanos() as u64
}

/// Set HOME to the given path and clear all API-key / config env vars.
/// Acquires the HOME_LOCK to prevent parallel tests from stomping on each
/// other. Returns a guard that restores the original HOME and releases the
/// lock on drop.
fn isolate_home(home: &PathBuf) -> EnvGuard {
    let _lock = HOME_LOCK.lock().expect("HOME_LOCK");
    let old_home = std::env::var_os("HOME");
    std::env::set_var("HOME", home);
    for key in &[
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "DEEPSEEK_API_KEY",
        "PI_API_KEY",
        "PI_MODEL",
        "PI_PROVIDER",
        "PI_BASE_URL",
    ] {
        std::env::remove_var(key);
    }
    EnvGuard(old_home, _lock)
}

struct EnvGuard(Option<OsString>, std::sync::MutexGuard<'static, ()>);
impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.0 {
            Some(val) => std::env::set_var("HOME", val),
            None => std::env::remove_var("HOME"),
        }
        // lock guard dropped here
    }
}

// ---------------------------------------------------------------------------
// Config write_global / read roundtrip
// ---------------------------------------------------------------------------

#[test]
fn write_global_produces_valid_toml() {
    let (home, guard) = temp_home();
    let _env = isolate_home(&home);

    let config = Config {
        llm: LlmConfig {
            model: "claude-sonnet-5".into(),
            provider: "anthropic".into(),
            api_key: "sk-test-key".into(),
            base_url: "https://api.anthropic.com".into(),
        },
    };

    config::write_global(config.clone()).expect("write_global");

    // Verify the file exists at the expected path
    let path = config::global_config_path();
    assert!(path.exists(), "config file should exist at {:?}", path);

    // Read raw content and verify it is valid TOML
    let raw = std::fs::read_to_string(&path).expect("read file");
    let parsed: Config = toml::from_str(&raw).expect("parse TOML");

    assert_eq!(parsed.llm.model, config.llm.model);
    assert_eq!(parsed.llm.provider, config.llm.provider);
    assert_eq!(parsed.llm.api_key, config.llm.api_key);
    assert_eq!(parsed.llm.base_url, config.llm.base_url);

    drop(guard);
}

#[test]
fn write_global_creates_parent_directory() {
    let (home, guard) = temp_home();
    let _env = isolate_home(&home);

    // Ensure the .pi-oxide directory does not exist yet
    let config_dir = home.join(".pi-oxide");
    assert!(!config_dir.exists());

    let config = Config::default();
    config::write_global(config).expect("write_global");

    assert!(config_dir.exists(), "parent dir should be created");
    assert!(config::global_config_path().exists());

    drop(guard);
}

#[test]
fn write_global_overwrites_existing_config() {
    let (home, guard) = temp_home();
    let _env = isolate_home(&home);

    // Write first config
    let first = Config {
        llm: LlmConfig {
            model: "old-model".into(),
            provider: "openai".into(),
            api_key: "sk-old".into(),
            base_url: "https://old.api".into(),
        },
    };
    config::write_global(first).expect("write first");

    // Write second config
    let second = Config {
        llm: LlmConfig {
            model: "new-model".into(),
            provider: "anthropic".into(),
            api_key: "sk-new".into(),
            base_url: "https://new.api".into(),
        },
    };
    config::write_global(second).expect("write second");

    // Read back — should be the second config
    let (loaded, _) = config::load();
    assert_eq!(loaded.llm.model, "new-model");
    assert_eq!(loaded.llm.provider, "anthropic");
    assert_eq!(loaded.llm.api_key, "sk-new");
    assert_eq!(loaded.llm.base_url, "https://new.api");

    drop(guard);
}

#[test]
fn write_global_toml_contains_kebab_case_keys() {
    let (home, guard) = temp_home();
    let _env = isolate_home(&home);

    let config = Config {
        llm: LlmConfig {
            model: "gpt-5.5".into(),
            provider: "openai".into(),
            api_key: "sk-test".into(),
            base_url: "https://api.openai.com".into(),
        },
    };
    config::write_global(config).expect("write_global");

    let raw = std::fs::read_to_string(config::global_config_path()).expect("read");
    assert!(
        raw.contains("api-key"),
        "TOML should use kebab-case for api-key"
    );
    assert!(
        raw.contains("base-url"),
        "TOML should use kebab-case for base-url"
    );

    drop(guard);
}

// ---------------------------------------------------------------------------
// config::resolve picks up written config
// ---------------------------------------------------------------------------

#[test]
fn resolve_picks_up_global_config() {
    let (home, guard) = temp_home();
    let _env = isolate_home(&home);

    let config = Config {
        llm: LlmConfig {
            model: "deepseek-v4-flash".into(),
            provider: "deepseek".into(),
            api_key: "sk-deepseek-test".into(),
            base_url: "https://api.deepseek.com".into(),
        },
    };
    config::write_global(config).expect("write_global");

    let resolved = config::resolve(None, None, None, None);

    assert_eq!(resolved.model, "deepseek-v4-flash");
    assert_eq!(resolved.provider, "deepseek");
    assert_eq!(resolved.api_key, "sk-deepseek-test");
    assert_eq!(resolved.base_url, "https://api.deepseek.com");
    assert!(resolved.config_path.is_some());

    drop(guard);
}

#[test]
fn resolve_cli_overrides_config_file() {
    let (home, guard) = temp_home();
    let _env = isolate_home(&home);

    // Write a config with one set of values
    let config = Config {
        llm: LlmConfig {
            model: "config-model".into(),
            provider: "openai".into(),
            api_key: "sk-config".into(),
            base_url: "https://config.api".into(),
        },
    };
    config::write_global(config).expect("write_global");

    // Resolve with CLI overrides
    let resolved = config::resolve(
        Some("cli-model"),
        Some("anthropic"),
        Some("sk-cli"),
        Some("https://cli.api"),
    );

    assert_eq!(resolved.model, "cli-model");
    assert_eq!(resolved.provider, "anthropic");
    assert_eq!(resolved.api_key, "sk-cli");
    assert_eq!(resolved.base_url, "https://cli.api");

    drop(guard);
}

#[test]
fn resolve_env_overrides_config_file() {
    let (home, guard) = temp_home();
    let _env = isolate_home(&home);

    let config = Config {
        llm: LlmConfig {
            model: "config-model".into(),
            provider: "anthropic".into(),
            api_key: "sk-config".into(),
            base_url: "https://config.api".into(),
        },
    };
    config::write_global(config).expect("write_global");

    // Set env var — must be done after isolate_home clears it
    std::env::set_var("ANTHROPIC_API_KEY", "sk-env");

    let resolved = config::resolve(None, None, None, None);

    // API key should come from env, not config
    assert_eq!(resolved.api_key, "sk-env");
    // Other fields from config
    assert_eq!(resolved.model, "config-model");
    assert_eq!(resolved.provider, "anthropic");

    drop(guard);
}

#[test]
fn resolve_uses_hardcoded_defaults_when_no_config() {
    let (_home, guard) = temp_home();
    let _env = isolate_home(&_home);

    let resolved = config::resolve(None, None, None, None);

    assert_eq!(resolved.model, "claude-sonnet-5");
    assert_eq!(resolved.provider, "anthropic");
    assert_eq!(resolved.base_url, "https://api.anthropic.com");
    assert!(resolved.api_key.is_empty());
    assert!(resolved.config_path.is_none());

    drop(guard);
}

// ---------------------------------------------------------------------------
// Custom provider flow produces expected wire format
// ---------------------------------------------------------------------------

#[test]
fn custom_provider_openai_compat() {
    // Simulates what onboarding returns for a custom openai-compat provider
    let result = OnboardingResult {
        provider: "openai-compat".into(),
        model: "my-custom-model".into(),
        api_key: "sk-custom".into(),
        base_url: "https://custom.api.example".into(),
    };

    assert_eq!(result.provider, "openai-compat");
    assert_eq!(result.model, "my-custom-model");
    assert_eq!(result.base_url, "https://custom.api.example");

    // Verify this config can be written and resolved
    let (home, guard) = temp_home();
    let _env = isolate_home(&home);

    let config = Config {
        llm: LlmConfig {
            model: result.model.clone(),
            provider: result.provider.clone(),
            api_key: result.api_key.clone(),
            base_url: result.base_url.clone(),
        },
    };
    config::write_global(config).expect("write_global");

    let resolved = config::resolve(None, None, None, None);
    assert_eq!(resolved.provider, "openai-compat");
    assert_eq!(resolved.model, "my-custom-model");
    assert_eq!(resolved.base_url, "https://custom.api.example");

    drop(guard);
}

#[test]
fn custom_provider_anthropic_compat() {
    let result = OnboardingResult {
        provider: "anthropic-compat".into(),
        model: "custom-claude".into(),
        api_key: "sk-custom".into(),
        base_url: "https://custom.anthropic.api".into(),
    };

    assert_eq!(result.provider, "anthropic-compat");
    assert_eq!(result.model, "custom-claude");

    let (home, guard) = temp_home();
    let _env = isolate_home(&home);

    let config = Config {
        llm: LlmConfig {
            model: result.model.clone(),
            provider: result.provider.clone(),
            api_key: result.api_key.clone(),
            base_url: result.base_url.clone(),
        },
    };
    config::write_global(config).expect("write_global");

    let resolved = config::resolve(None, None, None, None);
    assert_eq!(resolved.provider, "anthropic-compat");
    assert_eq!(resolved.model, "custom-claude");

    drop(guard);
}

// ---------------------------------------------------------------------------
// Full onboarding simulation: write config, verify resolve
// ---------------------------------------------------------------------------

#[test]
fn full_onboarding_flow_anthropic() {
    let (home, guard) = temp_home();
    let _env = isolate_home(&home);

    // Simulate the onboarding result for Anthropic preset
    let onboard = OnboardingResult {
        provider: "anthropic".into(),
        model: "claude-sonnet-5".into(),
        api_key: "sk-ant-test-key".into(),
        base_url: "https://api.anthropic.com".into(),
    };

    // Write config (as onboarding::run() does)
    let config = Config {
        llm: LlmConfig {
            model: onboard.model.clone(),
            provider: onboard.provider.clone(),
            api_key: onboard.api_key.clone(),
            base_url: onboard.base_url.clone(),
        },
    };
    config::write_global(config).expect("write_global");

    // Verify config file exists
    assert!(config::global_config_path().exists());

    // Verify resolve picks it up
    let resolved = config::resolve(None, None, None, None);
    assert_eq!(resolved.model, "claude-sonnet-5");
    assert_eq!(resolved.provider, "anthropic");
    assert_eq!(resolved.api_key, "sk-ant-test-key");
    assert_eq!(resolved.base_url, "https://api.anthropic.com");

    drop(guard);
}

#[test]
fn full_onboarding_flow_openai() {
    let (home, guard) = temp_home();
    let _env = isolate_home(&home);

    let onboard = OnboardingResult {
        provider: "openai".into(),
        model: "gpt-5.5".into(),
        api_key: "sk-oai-test-key".into(),
        base_url: "https://api.openai.com".into(),
    };

    let config = Config {
        llm: LlmConfig {
            model: onboard.model.clone(),
            provider: onboard.provider.clone(),
            api_key: onboard.api_key.clone(),
            base_url: onboard.base_url.clone(),
        },
    };
    config::write_global(config).expect("write_global");

    let resolved = config::resolve(None, None, None, None);
    assert_eq!(resolved.model, "gpt-5.5");
    assert_eq!(resolved.provider, "openai");
    assert_eq!(resolved.api_key, "sk-oai-test-key");
    assert_eq!(resolved.base_url, "https://api.openai.com");

    drop(guard);
}

#[test]
fn full_onboarding_flow_deepseek() {
    let (home, guard) = temp_home();
    let _env = isolate_home(&home);

    let onboard = OnboardingResult {
        provider: "deepseek".into(),
        model: "deepseek-v4-flash".into(),
        api_key: "sk-ds-test-key".into(),
        base_url: "https://api.deepseek.com".into(),
    };

    let config = Config {
        llm: LlmConfig {
            model: onboard.model.clone(),
            provider: onboard.provider.clone(),
            api_key: onboard.api_key.clone(),
            base_url: onboard.base_url.clone(),
        },
    };
    config::write_global(config).expect("write_global");

    let resolved = config::resolve(None, None, None, None);
    assert_eq!(resolved.model, "deepseek-v4-flash");
    assert_eq!(resolved.provider, "deepseek");
    assert_eq!(resolved.api_key, "sk-ds-test-key");
    assert_eq!(resolved.base_url, "https://api.deepseek.com");

    drop(guard);
}

// ---------------------------------------------------------------------------
// Config file presence controls onboarding decision
// ---------------------------------------------------------------------------

#[test]
fn config_file_presence_skips_onboarding() {
    let (home, guard) = temp_home();
    let _env = isolate_home(&home);

    // No config — onboarding would run
    assert!(!config::global_config_path().exists());

    // Write a config — onboarding should now be skipped
    config::write_global(Config::default()).expect("write_global");
    assert!(config::global_config_path().exists());

    drop(guard);
}

#[test]
fn no_config_file_means_onboarding_needed() {
    let (_home, guard) = temp_home();
    let _env = isolate_home(&_home);

    // Fresh home — no config file exists
    assert!(!config::global_config_path().exists());

    // resolve returns defaults with no config path
    let resolved = config::resolve(None, None, None, None);
    assert!(resolved.config_path.is_none());
    assert!(resolved.api_key.is_empty());

    drop(guard);
}
