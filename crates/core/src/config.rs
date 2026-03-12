use anyhow::Context;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub listen: String,
    pub providers: HashMap<String, ProviderConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: "127.0.0.1:8080".to_string(),
            providers: HashMap::new(),
        }
    }
}

pub const CONFIG_ENV_VAR: &str = "CONDUIT_CONFIG";

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
        config.normalize();
        Ok(config)
    }

    fn normalize(&mut self) {
        if self.listen.starts_with(':') {
            self.listen = format!("127.0.0.1{}", self.listen);
        }
        for provider in self.providers.values_mut() {
            provider.upstream = provider.upstream.trim_end_matches('/').to_string();
        }
    }
}

#[derive(Debug, Deserialize, PartialEq)]
pub struct ProviderConfig {
    pub upstream: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_listen() {
        let toml = r#"
listen = "127.0.0.1:9090"
"#;
        let config = Config::from_toml_str(toml).unwrap();
        assert_eq!(config.listen, "127.0.0.1:9090");
        assert!(config.providers.is_empty());
    }

    #[test]
    fn parse_providers() {
        let toml = r#"
[providers.openai]
upstream = "https://api.openai.com"

[providers.anthropic]
upstream = "https://api.anthropic.com"
"#;
        let config = Config::from_toml_str(toml).unwrap();
        assert_eq!(config.providers.len(), 2);
        assert_eq!(
            config.providers["openai"].upstream,
            "https://api.openai.com"
        );
        assert_eq!(
            config.providers["anthropic"].upstream,
            "https://api.anthropic.com"
        );
    }

    #[test]
    fn listen_shorthand_port_only() {
        let config = Config::from_toml_str(r#"listen = ":9090""#).unwrap();
        assert_eq!(config.listen, "127.0.0.1:9090");
    }

    #[test]
    fn defaults_when_empty() {
        let config = Config::from_toml_str("").unwrap();
        assert_eq!(config.listen, "127.0.0.1:8080");
        assert!(config.providers.is_empty());
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
    fn upstream_trailing_slash_stripped() {
        let toml = r#"
[providers.openai]
upstream = "https://api.openai.com/"
"#;
        let config = Config::from_toml_str(toml).unwrap();
        assert_eq!(
            config.providers["openai"].upstream,
            "https://api.openai.com"
        );
    }

    #[test]
    fn load_from_env_var() {
        let dir = std::env::temp_dir().join("conduit-test-env");
        std::fs::create_dir_all(&dir).unwrap();
        let config_path = dir.join("conduit-env.toml");
        std::fs::write(&config_path, r#"listen = "127.0.0.1:9999""#).unwrap();

        unsafe { std::env::set_var(CONFIG_ENV_VAR, &config_path) };
        let config = Config::load(None).unwrap();
        unsafe { std::env::remove_var(CONFIG_ENV_VAR) };

        assert_eq!(config.listen, "127.0.0.1:9999");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cli_flag_overrides_env_var() {
        let dir = std::env::temp_dir().join("conduit-test-override");
        std::fs::create_dir_all(&dir).unwrap();
        let env_path = dir.join("env.toml");
        let cli_path = dir.join("cli.toml");
        std::fs::write(&env_path, r#"listen = "127.0.0.1:1111""#).unwrap();
        std::fs::write(&cli_path, r#"listen = "127.0.0.1:2222""#).unwrap();

        unsafe { std::env::set_var(CONFIG_ENV_VAR, &env_path) };
        let config = Config::load(Some(cli_path)).unwrap();
        unsafe { std::env::remove_var(CONFIG_ENV_VAR) };

        assert_eq!(config.listen, "127.0.0.1:2222");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
