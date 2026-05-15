pub mod config;

use std::sync::Arc;

use roder_api::catalog::{DEFAULT_MODEL_ID, PROVIDER_MOCK};
use roder_app_server::{AppServer, LocalAppClient};
use roder_core::{Runtime, RuntimeConfig};
use roder_extension_host::{DefaultRegistryConfig, build_default_registry};
use roder_tui::TuiApp;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = config::load_config().unwrap_or_default();

    let openai_key = std::env::var("OPENAI_API_KEY")
        .ok()
        .or_else(|| cfg.providers.get("openai").and_then(|p| p.api_key.clone()))
        .or_else(|| {
            cfg.providers
                .get("openai_responses")
                .and_then(|p| p.api_key.clone())
        });
    let anthropic_key = std::env::var("ANTHROPIC_API_KEY").ok().or_else(|| {
        cfg.providers
            .get("anthropic")
            .and_then(|p| p.api_key.clone())
    });
    let gemini_key = std::env::var("GEMINI_API_TOKEN")
        .ok()
        .or_else(|| std::env::var("GEMINI_API_KEY").ok())
        .or_else(|| std::env::var("GOOGLE_API_KEY").ok())
        .or_else(|| std::env::var("GOOGLE_GENAI_API_KEY").ok())
        .or_else(|| std::env::var("GOOGLE_AI_API_KEY").ok())
        .or_else(|| cfg.providers.get("gemini").and_then(|p| p.api_key.clone()));

    let registry = build_default_registry(DefaultRegistryConfig {
        openai_api_key: openai_key,
        anthropic_api_key: anthropic_key,
        gemini_api_key: gemini_key,
        session_dir: None,
    })?;

    let default_provider = cfg.provider.unwrap_or_else(|| PROVIDER_MOCK.to_string());
    let default_model = cfg.model.unwrap_or_else(|| {
        if default_provider == PROVIDER_MOCK {
            "mock".to_string()
        } else {
            DEFAULT_MODEL_ID.to_string()
        }
    });

    let runtime = Arc::new(Runtime::new(
        registry,
        RuntimeConfig {
            default_provider,
            default_model: default_model.clone(),
            workspace: std::env::current_dir()
                .ok()
                .map(|p| p.display().to_string()),
        },
    )?);
    let app_server = Arc::new(AppServer::new(runtime));
    let client = LocalAppClient::new(app_server);

    let mut tui = TuiApp::new(client, default_model).await?;
    tui.run().await?;
    Ok(())
}
