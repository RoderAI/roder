use std::sync::Arc;

use roder_api::catalog::{DEFAULT_MODEL_ID, PROVIDER_MOCK};
use roder_app_server::{AppServer, LocalAppClient};
use roder_core::{Runtime, RuntimeConfig};
use roder_extension_host::{DefaultRegistryConfig, build_default_registry};
use roder_tui::TuiApp;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if matches!(args.first().map(String::as_str), Some("auth")) {
        return run_auth(&args[1..]).await;
    }

    let cfg = roder_config::load_config().unwrap_or_default();

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

    let (default_provider, configured_model) = resolve_provider_model(cfg.provider, cfg.model);

    let registry = build_default_registry(DefaultRegistryConfig {
        openai_api_key: openai_key,
        anthropic_api_key: anthropic_key,
        gemini_api_key: gemini_key,
        session_dir: None,
    })?;

    let default_model = configured_model.unwrap_or_else(|| {
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
            reasoning: cfg.reasoning,
            auto_compact_token_limit: cfg.auto_compact_token_limit,
            workspace: std::env::current_dir()
                .ok()
                .map(|p| p.display().to_string()),
        },
    )?);
    let app_server = Arc::new(AppServer::new(runtime).with_user_config_persistence());
    let client = LocalAppClient::new(app_server);

    let mut tui = TuiApp::new(client, default_model).await?;
    tui.run().await?;
    Ok(())
}

async fn run_auth(args: &[String]) -> anyhow::Result<()> {
    match args.first().map(String::as_str) {
        Some("login") => {
            let provider = args.get(1).map(String::as_str).unwrap_or("codex");
            if provider != "codex" {
                anyhow::bail!("unsupported auth provider {provider:?}");
            }
            eprintln!("Opening browser for Codex sign-in...");
            let tokens = roder_codex_auth::login().await?;
            if tokens.account_id.is_empty() {
                eprintln!("Signed in with Codex");
            } else {
                eprintln!("Signed in with Codex account {}", tokens.account_id);
            }
            Ok(())
        }
        Some("status") => {
            match roder_codex_auth::status().await? {
                Some(tokens) if !tokens.account_id.is_empty() => {
                    println!("codex: signed in ({})", tokens.account_id);
                }
                Some(_) => println!("codex: signed in"),
                None => println!("codex: signed out"),
            }
            Ok(())
        }
        Some("logout") => {
            roder_codex_auth::logout()?;
            println!("codex: signed out");
            Ok(())
        }
        _ => anyhow::bail!("usage: roder auth login codex|status|logout"),
    }
}

fn resolve_provider_model(
    provider: Option<String>,
    model: Option<String>,
) -> (String, Option<String>) {
    let Some(provider) = provider else {
        return (PROVIDER_MOCK.to_string(), model);
    };
    if let Some((provider_id, model_id)) = provider.split_once('/') {
        let provider_id = provider_id.trim();
        let model_id = model_id.trim();
        if !provider_id.is_empty() && !model_id.is_empty() {
            return (
                provider_id.to_string(),
                model.or_else(|| Some(model_id.to_string())),
            );
        }
    }
    (provider, model)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_slash_model_sets_default_model() {
        let (provider, model) = resolve_provider_model(Some("codex/gpt-5.5".to_string()), None);
        assert_eq!(provider, "codex");
        assert_eq!(model.as_deref(), Some("gpt-5.5"));
    }

    #[test]
    fn explicit_model_wins_over_provider_slash_model() {
        let (provider, model) = resolve_provider_model(
            Some("codex/gpt-5.4-mini".to_string()),
            Some("gpt-5.5".to_string()),
        );
        assert_eq!(provider, "codex");
        assert_eq!(model.as_deref(), Some("gpt-5.5"));
    }
}
