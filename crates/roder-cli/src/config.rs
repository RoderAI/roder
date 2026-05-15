use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Config {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub reasoning: Option<String>,
    pub auto_compact_token_limit: Option<u32>,
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub api_key: Option<String>,
}

pub fn load_config() -> anyhow::Result<Config> {
    let mut config_path = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    config_path.push(".roder");
    config_path.push("config.toml");

    let mut config = if config_path.exists() {
        let contents = fs::read_to_string(config_path)?;
        toml::from_str(&contents)?
    } else {
        Config::default()
    };
    if let Ok(provider) = std::env::var("RODER_PROVIDER") {
        if !provider.trim().is_empty() {
            config.provider = Some(provider);
        }
    }
    if let Ok(model) = std::env::var("RODER_MODEL") {
        if !model.trim().is_empty() {
            config.model = Some(model);
        }
    }
    if let Ok(reasoning) = std::env::var("RODER_REASONING") {
        if !reasoning.trim().is_empty() {
            config.reasoning = Some(reasoning);
        }
    }
    if let Ok(limit) = std::env::var("RODER_AUTO_COMPACT_TOKEN_LIMIT") {
        if let Ok(limit) = limit.trim().parse::<u32>() {
            config.auto_compact_token_limit = Some(limit);
        }
    }
    Ok(config)
}
