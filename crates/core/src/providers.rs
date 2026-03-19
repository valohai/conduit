use std::fmt;
use std::str::FromStr;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Provider {
    OpenAI,
    Anthropic,
    #[default]
    Unknown,
}

impl Provider {
    pub fn detect(url: &str) -> Self {
        let lower = url.to_lowercase();
        if lower.contains("api.openai.com") {
            Self::OpenAI
        } else if lower.contains("api.anthropic.com") {
            Self::Anthropic
        } else {
            Self::Unknown
        }
    }
}

impl fmt::Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::OpenAI => "openai",
            Self::Anthropic => "anthropic",
            Self::Unknown => "unknown",
        };
        f.write_str(s)
    }
}

impl FromStr for Provider {
    type Err = std::convert::Infallible; // allow for `let Ok(provider) = ...`

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "openai" => Self::OpenAI,
            "anthropic" => Self::Anthropic,
            _ => Self::Unknown,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_openai() {
        assert_eq!(
            Provider::detect("https://api.openai.com/v1/responses"),
            Provider::OpenAI
        );
        assert_eq!(
            Provider::detect("https://api.openai.com/v1/chat/completions"),
            Provider::OpenAI
        );
    }

    #[test]
    fn detect_anthropic() {
        assert_eq!(
            Provider::detect("https://api.anthropic.com/v1/messages"),
            Provider::Anthropic
        );
    }

    #[test]
    fn detect_unknown() {
        assert_eq!(
            Provider::detect("https://my-custom-llm.example.com/v1"),
            Provider::Unknown
        );
    }

    #[test]
    fn display_roundtrip() {
        let providers = [Provider::OpenAI, Provider::Anthropic, Provider::Unknown];
        for provider in &providers {
            let s = provider.to_string();
            let Ok(parsed) = s.parse::<Provider>();
            assert_eq!(&parsed, provider);
        }
    }

    #[test]
    fn from_str_case_insensitive() {
        let Ok(openai) = "openai".parse::<Provider>();
        assert_eq!(openai, Provider::OpenAI);
        let Ok(anthropic) = "ANTHROPIC".parse::<Provider>();
        assert_eq!(anthropic, Provider::Anthropic);
    }

    #[test]
    fn from_str_unknown_fallback() {
        let Ok(provider) = "not_a_provider".parse::<Provider>();
        assert_eq!(provider, Provider::Unknown);
    }
}
