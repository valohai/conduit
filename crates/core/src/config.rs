use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub listen: String,
    pub database: PathBuf,
    pub providers: HashMap<String, ProviderConfig>,
    #[serde(rename = "valohai-llm")]
    pub valohai_llm: ValohaiLlmConfig,
    /// Model name prefix -> rates; checked before the built-in pricing table.
    pub pricing: HashMap<String, ModelPricing>,
    pub upstream: UpstreamConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: "127.0.0.1:8080".to_string(),
            database: PathBuf::from("conduit.db"),
            providers: HashMap::new(),
            valohai_llm: ValohaiLlmConfig::default(),
            pricing: HashMap::new(),
            upstream: UpstreamConfig::default(),
        }
    }
}

/// Timeouts for the requests forwarded to providers, in seconds; zero disables one.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default)]
pub struct UpstreamConfig {
    pub connect_timeout_s: u64,
    pub idle_timeout_s: u64,
}

impl Default for UpstreamConfig {
    fn default() -> Self {
        Self {
            connect_timeout_s: 10,
            // the same timeout bounds the wait for a unary response's headers, which can take
            // minutes on a long non-streaming request; there is no total deadline, as a stream
            // has no known length
            idle_timeout_s: 600,
        }
    }
}

impl UpstreamConfig {
    pub fn connect_timeout(&self) -> Option<Duration> {
        (self.connect_timeout_s > 0).then(|| Duration::from_secs(self.connect_timeout_s))
    }

    pub fn idle_timeout(&self) -> Option<Duration> {
        (self.idle_timeout_s > 0).then(|| Duration::from_secs(self.idle_timeout_s))
    }
}

pub const CONFIG_ENV_VAR: &str = "CONDUIT_CONFIG";
pub const DATABASE_ENV_VAR: &str = "CONDUIT_DATABASE";
pub const LLM_URL_ENV_VAR: &str = "VALOHAI_LLM_URL";
pub const LLM_API_KEY_ENV_VAR: &str = "VALOHAI_LLM_API_KEY";

impl Config {
    pub fn load(explicit_path: Option<PathBuf>) -> anyhow::Result<Self> {
        Self::load_with_env(explicit_path, |name| std::env::var(name).ok())
    }

    fn load_with_env(
        explicit_path: Option<PathBuf>,
        read_env_var: impl Fn(&str) -> Option<String>,
    ) -> anyhow::Result<Self> {
        match explicit_path.or_else(|| read_env_var(CONFIG_ENV_VAR).map(PathBuf::from)) {
            Some(path) => Self::from_path(&path, read_env_var),
            None => Self::from_path_if_exists(Path::new("conduit.toml"), read_env_var),
        }
    }

    fn from_path_if_exists(
        path: &Path,
        read_env_var: impl Fn(&str) -> Option<String>,
    ) -> anyhow::Result<Self> {
        match std::fs::read_to_string(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Ok(Self::from_toml_str("", read_env_var)?)
            }
            _ => Self::from_path(path, read_env_var),
        }
    }

    fn from_path(
        path: &Path,
        read_env_var: impl Fn(&str) -> Option<String>,
    ) -> anyhow::Result<Self> {
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        Self::from_toml_str(&contents, read_env_var)
            .with_context(|| format!("failed to parse {}", path.display()))
    }

    fn from_toml_str(
        s: &str,
        read_env_var: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, toml::de::Error> {
        let mut config: Config = toml::from_str(s)?;
        config.after_load(read_env_var);
        Ok(config)
    }

    fn after_load(&mut self, read_env_var: impl Fn(&str) -> Option<String>) {
        // apply env var overrides and normalize values

        if self.listen.starts_with(':') {
            self.listen = format!("127.0.0.1{}", self.listen);
        }
        for provider in self.providers.values_mut() {
            provider.upstream = provider.upstream.trim_end_matches('/').to_string();
        }

        self.apply_overrides_from_env_vars(read_env_var);
        self.valohai_llm.url = self.valohai_llm.url.trim_end_matches('/').to_string();
    }

    fn apply_overrides_from_env_vars(&mut self, read_env_var: impl Fn(&str) -> Option<String>) {
        let read_non_empty = |name| read_env_var(name).filter(|s| !s.is_empty());

        if let Some(database) = read_non_empty(DATABASE_ENV_VAR) {
            self.database = PathBuf::from(database);
        }
        if let Some(llm_url) = read_non_empty(LLM_URL_ENV_VAR) {
            self.valohai_llm.url = llm_url;
        }
        if let Some(llm_api_key) = read_non_empty(LLM_API_KEY_ENV_VAR) {
            self.valohai_llm.api_key = llm_api_key;
        }
    }
}

/// USD per million tokens, the same shape as an entry in `pricing.json`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ModelPricing {
    pub input: f64,
    pub output: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_input: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_5m: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_1h: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long_prompt: Option<LongPromptPricing>,
}

/// Rates for every token of a request whose prompt is longer than `tokens`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct LongPromptPricing {
    pub tokens: u64,
    pub input: f64,
    pub output: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_input: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_5m: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_1h: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ProviderConfig {
    pub upstream: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default)]
pub struct ValohaiLlmConfig {
    pub api_key: String,
    pub url: String,
}

impl Default for ValohaiLlmConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            url: "https://llm.valohai.com".to_string(),
        }
    }
}

impl ValohaiLlmConfig {
    pub fn enabled(&self) -> bool {
        !self.api_key.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_listen() -> anyhow::Result<()> {
        let toml = r#"
listen = "127.0.0.1:9090"
"#;
        let config = Config::from_toml_str(toml, |_| None)?;
        assert_eq!(config.listen, "127.0.0.1:9090");
        assert!(config.providers.is_empty());
        Ok(())
    }

    #[test]
    fn parse_providers() -> anyhow::Result<()> {
        let toml = r#"
[providers.openai]
upstream = "https://api.openai.com"

[providers.anthropic]
upstream = "https://api.anthropic.com"
"#;
        let config = Config::from_toml_str(toml, |_| None)?;
        assert_eq!(config.providers.len(), 2);
        assert_eq!(
            config.providers["openai"].upstream,
            "https://api.openai.com"
        );
        assert_eq!(
            config.providers["anthropic"].upstream,
            "https://api.anthropic.com"
        );
        Ok(())
    }

    #[test]
    fn listen_shorthand_port_only() -> anyhow::Result<()> {
        let config = Config::from_toml_str(r#"listen = ":9090""#, |_| None)?;
        assert_eq!(config.listen, "127.0.0.1:9090");
        Ok(())
    }

    #[test]
    fn defaults_when_empty() -> anyhow::Result<()> {
        let config = Config::from_toml_str("", |_| None)?;
        assert_eq!(config.listen, "127.0.0.1:8080");
        assert_eq!(config.database, Path::new("conduit.db"));
        assert!(config.providers.is_empty());
        assert_eq!(
            config.upstream.connect_timeout(),
            Some(Duration::from_secs(10))
        );
        assert_eq!(
            config.upstream.idle_timeout(),
            Some(Duration::from_secs(600))
        );
        Ok(())
    }

    #[test]
    fn parse_upstream_timeouts() -> anyhow::Result<()> {
        let config = Config::from_toml_str("[upstream]\nidle_timeout_s = 30")?;
        assert_eq!(
            config.upstream.connect_timeout(),
            Some(Duration::from_secs(10))
        );
        assert_eq!(
            config.upstream.idle_timeout(),
            Some(Duration::from_secs(30))
        );

        let config = Config::from_toml_str("[upstream]\nidle_timeout_s = 0")?;
        assert_eq!(config.upstream.idle_timeout(), None);
        Ok(())
    }

    #[test]
    fn parse_database() -> anyhow::Result<()> {
        let config =
            Config::from_toml_str(r#"database = "/var/lib/conduit/conduit.db""#, |_| None)?;
        assert_eq!(config.database, Path::new("/var/lib/conduit/conduit.db"));
        Ok(())
    }

    #[test]
    fn database_env_var_overrides_config_file() -> anyhow::Result<()> {
        let config =
            Config::from_toml_str(r#"database = "/var/lib/conduit/conduit.db""#, |name| {
                (name == DATABASE_ENV_VAR).then(|| "/tmp/conduit-test/conduit.db".to_string())
            })?;
        assert_eq!(config.database, Path::new("/tmp/conduit-test/conduit.db"));
        Ok(())
    }

    #[test]
    fn database_env_var_applies_without_config_file() -> anyhow::Result<()> {
        // tests run in the crate directory, which has no `conduit.toml`
        let config = Config::load_with_env(None, |name| {
            (name == DATABASE_ENV_VAR).then(|| "/tmp/conduit-test/conduit.db".to_string())
        })?;
        assert_eq!(config.database, Path::new("/tmp/conduit-test/conduit.db"));
        Ok(())
    }

    #[test]
    fn from_path_missing_file() {
        let result = Config::from_path(Path::new("/nonexistent/conduit.toml"), |_| None);
        assert!(result.is_err());
    }

    #[test]
    fn optional_config_file_defaults_only_when_missing() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("conduit.toml");

        let config = Config::from_path_if_exists(&path, |_| None)?;
        assert_eq!(config, Config::default());

        std::fs::write(&path, "listen = [[[invalid")?;
        assert!(Config::from_path_if_exists(&path, |_| None).is_err());
        Ok(())
    }

    #[test]
    fn parse_invalid_toml() {
        let result = Config::from_toml_str("listen = [[[invalid", |_| None);
        assert!(result.is_err());
    }

    #[test]
    fn upstream_trailing_slash_stripped() -> anyhow::Result<()> {
        let toml = r#"
[providers.openai]
upstream = "https://api.openai.com/"
"#;
        let config = Config::from_toml_str(toml, |_| None)?;
        assert_eq!(
            config.providers["openai"].upstream,
            "https://api.openai.com"
        );
        Ok(())
    }

    #[test]
    fn load_from_env_var() -> anyhow::Result<()> {
        let dir = std::env::temp_dir().join("conduit-test-env");
        std::fs::create_dir_all(&dir)?;
        let config_path = dir.join("conduit-env.toml");
        std::fs::write(&config_path, r#"listen = "127.0.0.1:9999""#)?;

        unsafe { std::env::set_var(CONFIG_ENV_VAR, &config_path) };
        let config = Config::load(None)?;
        unsafe { std::env::remove_var(CONFIG_ENV_VAR) };

        assert_eq!(config.listen, "127.0.0.1:9999");
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn valohai_llm_integration_disabled_by_default() -> anyhow::Result<()> {
        let config = Config::from_toml_str("", |_| None)?;
        assert!(!config.valohai_llm.enabled());
        Ok(())
    }

    #[test]
    fn valohai_llm_is_enabled_if_we_have_api_key() -> anyhow::Result<()> {
        let toml = r#"
[valohai-llm]
api_key = "MY_KEY_123"
"#;
        let config = Config::from_toml_str(toml, |_| None)?;
        assert_eq!(config.valohai_llm.api_key, "MY_KEY_123");
        assert_eq!(config.valohai_llm.url, "https://llm.valohai.com");
        assert!(config.valohai_llm.enabled());
        Ok(())
    }

    #[test]
    fn you_can_override_valohai_llm_endpoint() -> anyhow::Result<()> {
        let toml = r#"
[valohai-llm]
api_key = "MY_KEY_123"
url = "http://localhost:1234/"
"#;
        let config = Config::from_toml_str(toml, |_| None)?;
        assert_eq!(config.valohai_llm.api_key, "MY_KEY_123");
        assert_eq!(config.valohai_llm.url, "http://localhost:1234");
        assert!(config.valohai_llm.enabled());
        Ok(())
    }

    #[test]
    fn parse_pricing() -> anyhow::Result<()> {
        let toml = r#"
[pricing."jev-"]
input = 0.042
output = 0.0
"#;
        let config = Config::from_toml_str(toml, |_| None)?;
        assert_eq!(
            config.pricing["jev-"],
            ModelPricing {
                input: 0.042,
                output: 0.0,
                cached_input: None,
                cache_write_5m: None,
                cache_write_1h: None,
                long_prompt: None,
            }
        );
        Ok(())
    }

    #[test]
    fn config_path_cli_flag_overrides_config_path_env_var() -> anyhow::Result<()> {
        let dir = std::env::temp_dir().join("conduit-test-override");
        std::fs::create_dir_all(&dir)?;

        let env_path = dir.join("env.toml");
        std::fs::write(&env_path, r#"listen = "127.0.0.1:1111""#)?;

        let cli_path = dir.join("cli.toml");
        std::fs::write(&cli_path, r#"listen = "127.0.0.1:2222""#)?;

        unsafe { std::env::set_var(CONFIG_ENV_VAR, &env_path) };
        let config = Config::load(Some(cli_path))?;
        unsafe { std::env::remove_var(CONFIG_ENV_VAR) };

        assert_eq!(config.listen, "127.0.0.1:2222");
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }
}
