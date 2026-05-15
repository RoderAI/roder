use std::sync::Arc;

use roder_api::catalog::{DEFAULT_MODEL_ID, PROVIDER_MOCK};
use roder_app_server::{AppServer, LocalAppClient};
use roder_core::{Runtime, RuntimeConfig};
use roder_extension_host::{
    DefaultRegistryConfig, DefaultWebSearchConfig, DefaultWebSearchProviderConfig,
    build_default_registry,
};
use roder_tui::TuiApp;
use roder_web_search::WebSearchProviderKind;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if matches!(args.first().map(String::as_str), Some("auth")) {
        return run_auth(&args[1..]).await;
    }

    let (runtime, default_model) = build_runtime_from_config()?;
    let app_server = Arc::new(AppServer::new(runtime).with_user_config_persistence());
    let client = LocalAppClient::new(app_server);

    let mut tui = TuiApp::new(client, default_model).await?;
    tui.run().await?;
    Ok(())
}

fn build_runtime_from_config() -> anyhow::Result<(Arc<Runtime>, String)> {
    let cfg = roder_config::load_config().unwrap_or_default();
    let keys = provider_keys(&cfg);
    let web_search = cfg
        .web_search
        .as_ref()
        .map(resolve_web_search_config)
        .transpose()?;
    let (default_provider, configured_model) = resolve_provider_model(cfg.provider, cfg.model);

    let registry = build_default_registry(DefaultRegistryConfig {
        openai_api_key: keys.openai,
        anthropic_api_key: keys.anthropic,
        gemini_api_key: keys.gemini,
        session_dir: None,
        web_search,
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

    Ok((runtime, default_model))
}

struct ProviderKeys {
    openai: Option<String>,
    anthropic: Option<String>,
    gemini: Option<String>,
}

fn provider_keys(cfg: &roder_config::Config) -> ProviderKeys {
    ProviderKeys {
        openai: std::env::var("OPENAI_API_KEY")
            .ok()
            .or_else(|| cfg.providers.get("openai").and_then(|p| p.api_key.clone()))
            .or_else(|| {
                cfg.providers
                    .get("openai_responses")
                    .and_then(|p| p.api_key.clone())
            }),
        anthropic: std::env::var("ANTHROPIC_API_KEY").ok().or_else(|| {
            cfg.providers
                .get("anthropic")
                .and_then(|p| p.api_key.clone())
        }),
        gemini: std::env::var("GEMINI_API_TOKEN")
            .ok()
            .or_else(|| std::env::var("GEMINI_API_KEY").ok())
            .or_else(|| std::env::var("GOOGLE_API_KEY").ok())
            .or_else(|| std::env::var("GOOGLE_GENAI_API_KEY").ok())
            .or_else(|| std::env::var("GOOGLE_AI_API_KEY").ok())
            .or_else(|| cfg.providers.get("gemini").and_then(|p| p.api_key.clone())),
    }
}

fn resolve_web_search_config(
    cfg: &roder_config::WebSearchConfig,
) -> anyhow::Result<DefaultWebSearchConfig> {
    let provider = match cfg.provider.as_deref() {
        Some(provider) => Some(parse_web_search_provider(provider)?),
        None => None,
    };
    Ok(DefaultWebSearchConfig {
        enabled: cfg.enabled,
        provider,
        firecrawl: resolve_web_search_provider_config(
            &cfg.firecrawl,
            "FIRECRAWL_API_KEY",
            "FIRECRAWL_BASE_URL",
            None,
        ),
        perplexity: resolve_web_search_provider_config(
            &cfg.perplexity,
            "PERPLEXITY_API_KEY",
            "PERPLEXITY_BASE_URL",
            None,
        ),
        tavily: resolve_web_search_provider_config(
            &cfg.tavily,
            "TAVILY_API_KEY",
            "TAVILY_BASE_URL",
            Some("TAVILY_PROJECT"),
        ),
        parallel: resolve_web_search_provider_config(
            &cfg.parallel,
            "PARALLEL_API_KEY",
            "PARALLEL_BASE_URL",
            None,
        ),
        timeout_seconds: cfg.timeout_seconds,
        max_results: cfg.max_results,
        namespaced_tools: cfg.namespaced_tools,
    })
}

fn parse_web_search_provider(provider: &str) -> anyhow::Result<WebSearchProviderKind> {
    match provider.trim().to_ascii_lowercase().as_str() {
        "firecrawl" => Ok(WebSearchProviderKind::Firecrawl),
        "perplexity" => Ok(WebSearchProviderKind::Perplexity),
        "tavily" => Ok(WebSearchProviderKind::Tavily),
        "parallel" | "parallel.ai" | "parallel_ai" => Ok(WebSearchProviderKind::Parallel),
        _ => anyhow::bail!(
            "unsupported web_search provider {provider:?}; expected firecrawl, perplexity, tavily, or parallel"
        ),
    }
}

fn resolve_web_search_provider_config(
    cfg: &roder_config::WebSearchProviderConfig,
    default_api_key_env: &str,
    default_base_url_env: &str,
    default_project_env: Option<&str>,
) -> DefaultWebSearchProviderConfig {
    let api_key_env = cfg.api_key_env.as_deref().unwrap_or(default_api_key_env);
    let base_url_env = default_base_url_env;
    let project_env = cfg.project_env.as_deref().or(default_project_env);
    DefaultWebSearchProviderConfig {
        enabled: cfg.enabled,
        api_key: trim_nonempty(cfg.api_key.clone()).or_else(|| env_nonempty(api_key_env)),
        base_url: trim_nonempty(cfg.base_url.clone()).or_else(|| env_nonempty(base_url_env)),
        project_id: trim_nonempty(cfg.project.clone())
            .or_else(|| project_env.and_then(env_nonempty)),
        search_depth: trim_nonempty(cfg.search_depth.clone()),
        mode: trim_nonempty(cfg.mode.clone()),
        debug_raw_response: cfg.debug_raw_response,
    }
}

fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .and_then(|value| trim_nonempty(Some(value)))
}

fn trim_nonempty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
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
