use anyhow::Context;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub listen: String,
    pub providers: HashMap<String, ProviderConfig>,
    #[serde(rename = "valohai-llm")]
    pub valohai_llm: ValohaiLlmConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: "127.0.0.1:8080".to_string(),
            providers: HashMap::new(),
            valohai_llm: ValohaiLlmConfig::default(),
        }
    }
}

pub const CONFIG_ENV_VAR: &str = "CONDUIT_CONFIG";
pub const LLM_URL_ENV_VAR: &str = "VALOHAI_LLM_URL";
pub const LLM_API_KEY_ENV_VAR: &str = "VALOHAI_LLM_API_KEY";

impl Config {
    pub fn load(explicit_path: Option<PathBuf>) -> anyhow::Result<Self> {
        match explicit_path.or_else(|| std::env::var(CONFIG_ENV_VAR).ok().map(PathBuf::from)) {
            Some(path) => Self::from_path(&path),
            None => match Self::from_path(Path::new("conduit.toml")) {
                Ok(config) => Ok(config),
                Err(_) => Ok(Self::default()),
            },
        }
    }

    fn from_path(path: &Path) -> anyhow::Result<Self> {
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        Self::from_toml_str(&contents).map_err(Into::into)
    }

    fn from_toml_str(s: &str) -> Result<Self, toml::de::Error> {
        let mut config: Config = toml::from_str(s)?;
        config.post_load();
        Ok(config)
    }

    fn post_load(&mut self) {
        // apply env var overrides, then normalize values

        if self.listen.starts_with(':') {
            self.listen = format!("127.0.0.1{}", self.listen);
        }
        for provider in self.providers.values_mut() {
            provider.upstream = provider.upstream.trim_end_matches('/').to_string();
        }

        self.apply_overrides_from_env_vars();
        self.valohai_llm.url = self.valohai_llm.url.trim_end_matches('/').to_string();
    }

    fn apply_overrides_from_env_vars(&mut self) {
        let env_llm_url = std::env::var(LLM_URL_ENV_VAR)
            .ok()
            .filter(|s| !s.is_empty());
        if let Some(llm_url) = env_llm_url {
            self.valohai_llm.url = llm_url;
        }

        let env_llm_api_key = std::env::var(LLM_API_KEY_ENV_VAR)
            .ok()
            .filter(|s| !s.is_empty());
        if let Some(llm_api_key) = env_llm_api_key {
            self.valohai_llm.api_key = llm_api_key;
        }
    }
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
        let config = Config::from_toml_str(toml)?;
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
        let config = Config::from_toml_str(toml)?;
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
        let config = Config::from_toml_str(r#"listen = ":9090""#)?;
        assert_eq!(config.listen, "127.0.0.1:9090");
        Ok(())
    }

    #[test]
    fn defaults_when_empty() -> anyhow::Result<()> {
        let config = Config::from_toml_str("")?;
        assert_eq!(config.listen, "127.0.0.1:8080");
        assert!(config.providers.is_empty());
        Ok(())
    }

    #[test]
    fn from_path_missing_file() {
        let result = Config::from_path(Path::new("/nonexistent/conduit.toml"));
        assert!(result.is_err());
    }

    #[test]
    fn parse_invalid_toml() {
        let result = Config::from_toml_str("listen = [[[invalid");
        assert!(result.is_err());
    }

    #[test]
    fn upstream_trailing_slash_stripped() -> anyhow::Result<()> {
        let toml = r#"
[providers.openai]
upstream = "https://api.openai.com/"
"#;
        let config = Config::from_toml_str(toml)?;
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
        let config = Config::from_toml_str("")?;
        assert!(!config.valohai_llm.enabled());
        Ok(())
    }

    #[test]
    fn valohai_llm_is_enabled_if_we_have_api_key() -> anyhow::Result<()> {
        let toml = r#"
[valohai-llm]
api_key = "MY_KEY_123"
"#;
        let config = Config::from_toml_str(toml)?;
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
        let config = Config::from_toml_str(toml)?;
        assert_eq!(config.valohai_llm.api_key, "MY_KEY_123");
        assert_eq!(config.valohai_llm.url, "http://localhost:1234");
        assert!(config.valohai_llm.enabled());
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
