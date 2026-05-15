use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Config {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub reasoning: Option<String>,
    pub auto_compact_token_limit: Option<u32>,
    pub web_search: Option<WebSearchConfig>,
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WebSearchConfig {
    #[serde(default)]
    pub enabled: bool,
    pub provider: Option<String>,
    #[serde(default)]
    pub canonical_tool: bool,
    #[serde(default)]
    pub namespaced_tools: bool,
    pub max_results: Option<u8>,
    pub timeout_seconds: Option<u64>,
    #[serde(default)]
    pub firecrawl: WebSearchProviderConfig,
    #[serde(default)]
    pub perplexity: WebSearchProviderConfig,
    #[serde(default)]
    pub tavily: WebSearchProviderConfig,
    #[serde(default)]
    pub parallel: WebSearchProviderConfig,
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: None,
            canonical_tool: true,
            namespaced_tools: false,
            max_results: None,
            timeout_seconds: None,
            firecrawl: WebSearchProviderConfig::default(),
            perplexity: WebSearchProviderConfig::default(),
            tavily: WebSearchProviderConfig::default(),
            parallel: WebSearchProviderConfig::default(),
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WebSearchProviderConfig {
    #[serde(default)]
    pub enabled: bool,
    pub api_key: Option<String>,
    pub api_key_env: Option<String>,
    pub base_url: Option<String>,
    pub project: Option<String>,
    pub project_env: Option<String>,
    pub search_depth: Option<String>,
    pub mode: Option<String>,
    #[serde(default)]
    pub debug_raw_response: bool,
}

pub fn load_config() -> anyhow::Result<Config> {
    let mut config = load_config_file()?;
    apply_env_overrides(&mut config);
    Ok(config)
}

pub fn save_default_provider_model(provider: &str, model: &str) -> anyhow::Result<()> {
    save_default_provider_model_to_path(config_path(), provider, model)
}

pub fn save_default_provider_model_to_path(
    path: impl AsRef<Path>,
    provider: &str,
    model: &str,
) -> anyhow::Result<()> {
    let path = path.as_ref();
    let mut config = load_config_file_from_path(path)?;
    config.provider = Some(provider.to_string());
    config.model = Some(model.to_string());
    save_config_file_to_path(path, &config)
}

fn load_config_file() -> anyhow::Result<Config> {
    load_config_file_from_path(config_path())
}

fn load_config_file_from_path(path: impl AsRef<Path>) -> anyhow::Result<Config> {
    let path = path.as_ref();
    if path.exists() {
        let contents = fs::read_to_string(path)?;
        Ok(toml::from_str(&contents)?)
    } else {
        Ok(Config::default())
    }
}

fn save_config_file_to_path(path: impl AsRef<Path>, config: &Config) -> anyhow::Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let contents = toml::to_string_pretty(config)?;
    fs::write(path, contents)?;
    Ok(())
}

fn config_path() -> PathBuf {
    let mut config_path = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    config_path.push(".roder");
    config_path.push("config.toml");
    config_path
}

fn apply_env_overrides(config: &mut Config) {
    if let Ok(provider) = std::env::var("RODER_PROVIDER")
        && !provider.trim().is_empty()
    {
        config.provider = Some(provider);
    }
    if let Ok(model) = std::env::var("RODER_MODEL")
        && !model.trim().is_empty()
    {
        config.model = Some(model);
    }
    if let Ok(reasoning) = std::env::var("RODER_REASONING")
        && !reasoning.trim().is_empty()
    {
        config.reasoning = Some(reasoning);
    }
    if let Ok(limit) = std::env::var("RODER_AUTO_COMPACT_TOKEN_LIMIT")
        && let Ok(limit) = limit.trim().parse::<u32>()
    {
        config.auto_compact_token_limit = Some(limit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_provider_and_model_without_dropping_provider_blocks() {
        let mut config = Config {
            provider: Some("codex".to_string()),
            model: Some("gpt-5.5".to_string()),
            reasoning: Some("medium".to_string()),
            auto_compact_token_limit: None,
            web_search: None,
            providers: HashMap::new(),
        };
        config.providers.insert(
            "openai".to_string(),
            ProviderConfig {
                api_key: Some("key".to_string()),
            },
        );

        let encoded = toml::to_string_pretty(&config).unwrap();
        assert!(encoded.contains("provider = \"codex\""));
        assert!(encoded.contains("model = \"gpt-5.5\""));
        assert!(encoded.contains("[providers.openai]"));
        assert!(encoded.contains("api_key = \"key\""));
    }

    #[test]
    fn deserializes_web_search_config() {
        let config: Config = toml::from_str(
            r#"
            [web_search]
            enabled = true
            provider = "tavily"
            namespaced_tools = true
            max_results = 8
            timeout_seconds = 20

            [web_search.tavily]
            enabled = true
            api_key_env = "TAVILY_API_KEY"
            project_env = "TAVILY_PROJECT"
            base_url = "https://api.tavily.com"
            search_depth = "basic"
            "#,
        )
        .unwrap();

        let web_search = config.web_search.unwrap();
        assert!(web_search.enabled);
        assert_eq!(web_search.provider.as_deref(), Some("tavily"));
        assert!(web_search.namespaced_tools);
        assert_eq!(web_search.max_results, Some(8));
        assert_eq!(
            web_search.tavily.api_key_env.as_deref(),
            Some("TAVILY_API_KEY")
        );
        assert_eq!(
            web_search.tavily.project_env.as_deref(),
            Some("TAVILY_PROJECT")
        );
    }

    #[test]
    fn save_default_provider_model_creates_parent_directory() {
        let path = std::env::temp_dir()
            .join(format!("roder-config-test-{}", std::process::id()))
            .join("nested")
            .join("config.toml");
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir_all(path.parent().unwrap().parent().unwrap());

        save_default_provider_model_to_path(&path, "codex", "gpt-5.5").unwrap();
        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("provider = \"codex\""));
        assert!(contents.contains("model = \"gpt-5.5\""));

        let _ = fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }
}
