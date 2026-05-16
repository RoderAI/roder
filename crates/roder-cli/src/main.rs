use std::path::{Path, PathBuf};
use std::sync::Arc;

use roder_api::catalog::{DEFAULT_MODEL_ID, PROVIDER_MOCK};
use roder_api::inference::HostedWebSearchConfig;
use roder_api::policy_mode::PolicyMode;
use roder_app_server::{AppServer, LocalAppClient};
use roder_core::{Runtime, RuntimeConfig, validate_edit_tool};
use roder_ext_subagents::{AgentLoadConfig, load_agent_definitions};
use roder_extension_host::{
    DefaultRegistryConfig, DefaultSubagentsConfig, DefaultWebSearchConfig,
    DefaultWebSearchProviderConfig, build_default_registry,
};
use roder_protocol::{JsonRpcError, JsonRpcRequest, JsonRpcResponse};
use roder_tui::TuiApp;
use roder_web_search::WebSearchProviderKind;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if matches!(args.first().map(String::as_str), Some("auth")) {
        return run_auth(&args[1..]).await;
    }
    if matches!(args.first().map(String::as_str), Some("app-server")) {
        return run_app_server(&args[1..]).await;
    }

    let cli_options = parse_cli_options(&args)?;
    let (runtime, default_model) = build_runtime_from_config(cli_options).await?;
    let app_server = Arc::new(AppServer::new(runtime).with_user_config_persistence());
    let client = LocalAppClient::new(app_server);

    let mut tui = TuiApp::new(client, default_model).await?;
    tui.run().await?;
    Ok(())
}

#[derive(Debug, Clone, Default)]
struct CliOptions {
    policy_mode: Option<PolicyMode>,
}

#[derive(Debug, Clone)]
struct AppServerOptions {
    listen: String,
    cli_options: CliOptions,
}

async fn build_runtime_from_config(options: CliOptions) -> anyhow::Result<(Arc<Runtime>, String)> {
    let cfg = roder_config::load_config()?;
    let keys = provider_keys(&cfg);
    let web_search = resolve_web_search_config(cfg.web_search.as_ref())?;
    let policy_mode = resolve_policy_mode(&options, &cfg)?;
    let (default_provider, configured_model) = resolve_provider_model(cfg.provider, cfg.model);
    let default_model = configured_model.clone().unwrap_or_else(|| {
        if default_provider == PROVIDER_MOCK {
            "mock".to_string()
        } else {
            DEFAULT_MODEL_ID.to_string()
        }
    });
    let subagents = resolve_subagents_config(
        cfg.subagents.as_ref(),
        default_provider.clone(),
        default_model.clone(),
    )
    .await?;
    let model_edit_tools = resolve_model_edit_tools(&cfg.models)?;
    if policy_mode == PolicyMode::Bypass
        && cfg
            .policy_modes
            .as_ref()
            .and_then(|policy| policy.warn_on_bypass)
            .unwrap_or(true)
    {
        eprintln!("warning: bypass policy mode is active; tool approvals are auto-approved");
    }

    let workspace = std::env::current_dir().ok();
    let registry = build_default_registry(DefaultRegistryConfig {
        openai_api_key: keys.openai,
        anthropic_api_key: keys.anthropic,
        gemini_api_key: keys.gemini,
        session_dir: None,
        workspace: workspace.clone(),
        web_search: web_search.external,
        subagents,
        policy_mode,
    })?;

    let runtime = Arc::new(Runtime::new(
        registry,
        RuntimeConfig {
            default_provider,
            default_model: default_model.clone(),
            reasoning: cfg.reasoning,
            auto_compact_token_limit: cfg.auto_compact_token_limit,
            hosted_web_search: web_search.hosted,
            model_edit_tools,
            workspace: workspace.map(|p| p.display().to_string()),
            policy_mode,
        },
    )?);

    Ok((runtime, default_model))
}

fn resolve_model_edit_tools(
    models: &std::collections::HashMap<String, roder_config::ModelConfig>,
) -> anyhow::Result<std::collections::HashMap<String, String>> {
    let mut edit_tools = std::collections::HashMap::new();
    for (model, cfg) in models {
        let Some(edit_tool) = cfg.edit_tool.as_deref().map(str::trim) else {
            continue;
        };
        if edit_tool.is_empty() {
            continue;
        }
        validate_edit_tool(edit_tool)?;
        edit_tools.insert(model.clone(), edit_tool.to_string());
    }
    Ok(edit_tools)
}

fn parse_cli_options(args: &[String]) -> anyhow::Result<CliOptions> {
    let mut options = CliOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--yolo" => options.policy_mode = Some(PolicyMode::Bypass),
            "--mode" => {
                let Some(mode) = args.get(i + 1) else {
                    anyhow::bail!("--mode requires a value");
                };
                options.policy_mode = Some(parse_policy_mode(mode)?);
                i += 1;
            }
            arg if arg.starts_with("--mode=") => {
                options.policy_mode = Some(parse_policy_mode(&arg["--mode=".len()..])?);
            }
            _ => {}
        }
        i += 1;
    }
    Ok(options)
}

fn parse_app_server_options(args: &[String]) -> anyhow::Result<AppServerOptions> {
    let mut listen = "stdio://".to_string();
    let mut passthrough = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--listen" => {
                let Some(value) = args.get(i + 1) else {
                    anyhow::bail!("--listen requires a value");
                };
                listen = value.clone();
                i += 1;
            }
            arg if arg.starts_with("--listen=") => {
                listen = arg["--listen=".len()..].to_string();
            }
            other => passthrough.push(other.to_string()),
        }
        i += 1;
    }

    Ok(AppServerOptions {
        listen,
        cli_options: parse_cli_options(&passthrough)?,
    })
}

async fn run_app_server(args: &[String]) -> anyhow::Result<()> {
    let options = parse_app_server_options(args)?;
    if options.listen != "stdio://" {
        anyhow::bail!(
            "unsupported app-server listen address {:?}; only stdio:// is currently supported",
            options.listen
        );
    }

    let (runtime, _) = build_runtime_from_config(options.cli_options).await?;
    let app_server = Arc::new(AppServer::new(runtime).with_user_config_persistence());
    run_stdio_app_server(app_server).await
}

async fn run_stdio_app_server(app_server: Arc<AppServer>) -> anyhow::Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<serde_json::Value>();
    let writer = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(message) = rx.recv().await {
            stdout
                .write_all(serde_json::to_string(&message)?.as_bytes())
                .await?;
            stdout.write_all(b"\n").await?;
            stdout.flush().await?;
        }
        anyhow::Ok(())
    });

    let stdin = BufReader::new(tokio::io::stdin());
    let mut lines = stdin.lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<JsonRpcRequest>(&line) {
            Ok(request) => app_server.handle_request(request).await,
            Err(err) => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: None,
                result: None,
                error: Some(JsonRpcError {
                    code: -32700,
                    message: format!("Parse error: {err}"),
                    data: None,
                }),
            },
        };
        if tx.send(serde_json::to_value(response)?).is_err() {
            break;
        }
    }
    drop(tx);
    writer.await??;
    Ok(())
}

fn parse_policy_mode(mode: &str) -> anyhow::Result<PolicyMode> {
    match mode.trim() {
        "default" => Ok(PolicyMode::Default),
        "accept_edits" | "accept-edits" => Ok(PolicyMode::AcceptEdits),
        "plan" => Ok(PolicyMode::Plan),
        "bypass" | "yolo" => Ok(PolicyMode::Bypass),
        other => anyhow::bail!(
            "unsupported policy mode {other:?}; expected default, accept_edits, plan, or bypass"
        ),
    }
}

fn resolve_policy_mode(
    options: &CliOptions,
    cfg: &roder_config::Config,
) -> anyhow::Result<PolicyMode> {
    if let Some(mode) = options.policy_mode {
        return Ok(mode);
    }
    cfg.policy_modes
        .as_ref()
        .and_then(|policy| policy.default.as_deref())
        .map(parse_policy_mode)
        .transpose()
        .map(|mode| mode.unwrap_or_default())
}

async fn resolve_subagents_config(
    cfg: Option<&roder_config::SubagentsConfig>,
    default_provider: String,
    default_model: String,
) -> anyhow::Result<Option<DefaultSubagentsConfig>> {
    let Some(cfg) = cfg else {
        return Ok(None);
    };
    if !cfg.enabled {
        return Ok(Some(DefaultSubagentsConfig {
            enabled: false,
            ..DefaultSubagentsConfig::default()
        }));
    }

    let load_config = AgentLoadConfig {
        user_dir: resolve_user_agent_dir(cfg),
        workspace_dir: resolve_workspace_agent_dir(cfg)?,
    };
    let definitions = load_agent_definitions(&load_config).await?;
    Ok(Some(DefaultSubagentsConfig {
        enabled: true,
        definitions,
        default_agent: trim_nonempty(cfg.default_agent.clone())
            .unwrap_or_else(|| DefaultSubagentsConfig::default().default_agent),
        default_provider: Some(default_provider),
        default_model,
        max_concurrent: cfg
            .max_concurrent
            .unwrap_or_else(|| DefaultSubagentsConfig::default().max_concurrent),
        max_depth: cfg
            .max_depth
            .unwrap_or_else(|| DefaultSubagentsConfig::default().max_depth),
        default_timeout_seconds: cfg
            .default_timeout_seconds
            .unwrap_or_else(|| DefaultSubagentsConfig::default().default_timeout_seconds),
        include_child_transcript: cfg.include_child_transcript,
        expose_per_type: cfg.expose_per_type,
    }))
}

fn resolve_user_agent_dir(cfg: &roder_config::SubagentsConfig) -> Option<PathBuf> {
    cfg.disk
        .user_dir
        .as_deref()
        .map(expand_tilde)
        .or_else(roder_ext_subagents::default_user_agent_dir)
}

fn resolve_workspace_agent_dir(
    cfg: &roder_config::SubagentsConfig,
) -> anyhow::Result<Option<PathBuf>> {
    if let Some(path) = cfg.disk.workspace_dir.as_deref() {
        return Ok(Some(expand_tilde(path)));
    }
    Ok(None)
}

fn expand_tilde(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if text == "~" {
        home_dir().unwrap_or_else(|| path.to_path_buf())
    } else if let Some(rest) = text.strip_prefix("~/") {
        home_dir()
            .map(|home| home.join(rest))
            .unwrap_or_else(|| path.to_path_buf())
    } else {
        path.to_path_buf()
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
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

#[derive(Debug, Clone)]
struct ResolvedWebSearchConfig {
    external: Option<DefaultWebSearchConfig>,
    hosted: HostedWebSearchConfig,
}

fn resolve_web_search_config(
    cfg: Option<&roder_config::WebSearchConfig>,
) -> anyhow::Result<ResolvedWebSearchConfig> {
    let Some(cfg) = cfg else {
        return Ok(ResolvedWebSearchConfig {
            external: None,
            hosted: HostedWebSearchConfig::cached(),
        });
    };

    match web_search_mode(cfg)? {
        ResolvedWebSearchMode::HostedCached => Ok(ResolvedWebSearchConfig {
            external: None,
            hosted: HostedWebSearchConfig::cached(),
        }),
        ResolvedWebSearchMode::HostedLive => Ok(ResolvedWebSearchConfig {
            external: None,
            hosted: HostedWebSearchConfig::live(),
        }),
        ResolvedWebSearchMode::External => Ok(ResolvedWebSearchConfig {
            external: Some(resolve_external_web_search_config(cfg, true)?),
            hosted: HostedWebSearchConfig::disabled(),
        }),
        ResolvedWebSearchMode::Disabled => Ok(ResolvedWebSearchConfig {
            external: None,
            hosted: HostedWebSearchConfig::disabled(),
        }),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResolvedWebSearchMode {
    HostedCached,
    HostedLive,
    External,
    Disabled,
}

fn web_search_mode(cfg: &roder_config::WebSearchConfig) -> anyhow::Result<ResolvedWebSearchMode> {
    match cfg.mode.as_deref().map(normalize_mode) {
        Some(mode) if matches!(mode.as_str(), "codex" | "hosted" | "native" | "cached") => {
            Ok(ResolvedWebSearchMode::HostedCached)
        }
        Some(mode) if mode == "live" => Ok(ResolvedWebSearchMode::HostedLive),
        Some(mode) if matches!(mode.as_str(), "external" | "router" | "local") => {
            Ok(ResolvedWebSearchMode::External)
        }
        Some(mode) if matches!(mode.as_str(), "disabled" | "off" | "none" | "false") => {
            Ok(ResolvedWebSearchMode::Disabled)
        }
        Some(mode) => anyhow::bail!(
            "unsupported web_search.mode {mode:?}; expected codex, hosted, cached, live, external, or disabled"
        ),
        None if cfg
            .provider
            .as_deref()
            .is_some_and(is_hosted_web_search_provider) =>
        {
            Ok(ResolvedWebSearchMode::HostedCached)
        }
        None if cfg.enabled || cfg.provider.is_some() => Ok(ResolvedWebSearchMode::External),
        None => Ok(ResolvedWebSearchMode::Disabled),
    }
}

fn is_hosted_web_search_provider(provider: &str) -> bool {
    matches!(
        normalize_mode(provider).as_str(),
        "codex" | "openai" | "hosted" | "native"
    )
}

fn normalize_mode(mode: &str) -> String {
    mode.trim().to_ascii_lowercase().replace(['-', '_'], "")
}

fn resolve_external_web_search_config(
    cfg: &roder_config::WebSearchConfig,
    force_enabled: bool,
) -> anyhow::Result<DefaultWebSearchConfig> {
    let provider = match cfg.provider.as_deref() {
        Some(provider) => Some(parse_web_search_provider(provider)?),
        None => None,
    };
    Ok(DefaultWebSearchConfig {
        enabled: force_enabled || cfg.enabled,
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

    #[test]
    fn parses_policy_mode_cli_flags() {
        let options = parse_cli_options(&["--mode".to_string(), "plan".to_string()]).unwrap();
        assert_eq!(options.policy_mode, Some(PolicyMode::Plan));

        let options = parse_cli_options(&["--mode=accept-edits".to_string()]).unwrap();
        assert_eq!(options.policy_mode, Some(PolicyMode::AcceptEdits));

        let options = parse_cli_options(&["--yolo".to_string()]).unwrap();
        assert_eq!(options.policy_mode, Some(PolicyMode::Bypass));
    }

    #[test]
    fn config_policy_mode_is_validated() {
        let cfg = roder_config::Config {
            policy_modes: Some(roder_config::PolicyModesConfig {
                default: Some("plna".to_string()),
                ..roder_config::PolicyModesConfig::default()
            }),
            ..roder_config::Config::default()
        };

        let err = resolve_policy_mode(&CliOptions::default(), &cfg).unwrap_err();
        assert!(err.to_string().contains("unsupported policy mode"));
    }

    #[test]
    fn web_search_defaults_to_codex_hosted_cached() {
        let resolved = resolve_web_search_config(None).unwrap();

        assert!(resolved.external.is_none());
        assert_eq!(
            resolved.hosted.mode,
            roder_api::inference::HostedWebSearchMode::Cached
        );
    }

    #[test]
    fn web_search_live_mode_uses_codex_hosted_live() {
        let cfg = roder_config::WebSearchConfig {
            mode: Some("live".to_string()),
            ..roder_config::WebSearchConfig::default()
        };

        let resolved = resolve_web_search_config(Some(&cfg)).unwrap();

        assert!(resolved.external.is_none());
        assert_eq!(
            resolved.hosted.mode,
            roder_api::inference::HostedWebSearchMode::Live
        );
    }

    #[test]
    fn web_search_external_mode_uses_local_router() {
        let cfg = roder_config::WebSearchConfig {
            mode: Some("external".to_string()),
            provider: Some("tavily".to_string()),
            tavily: roder_config::WebSearchProviderConfig {
                api_key: Some("secret".to_string()),
                ..roder_config::WebSearchProviderConfig::default()
            },
            ..roder_config::WebSearchConfig::default()
        };

        let resolved = resolve_web_search_config(Some(&cfg)).unwrap();

        assert!(resolved.external.is_some());
        assert_eq!(
            resolved.hosted.mode,
            roder_api::inference::HostedWebSearchMode::Disabled
        );
    }

    #[test]
    fn web_search_disabled_mode_disables_hosted_and_external() {
        let cfg = roder_config::WebSearchConfig {
            mode: Some("disabled".to_string()),
            ..roder_config::WebSearchConfig::default()
        };

        let resolved = resolve_web_search_config(Some(&cfg)).unwrap();

        assert!(resolved.external.is_none());
        assert_eq!(
            resolved.hosted.mode,
            roder_api::inference::HostedWebSearchMode::Disabled
        );
    }

    #[tokio::test]
    async fn subagents_config_loads_agent_definitions_from_disk() {
        let root = std::env::temp_dir()
            .join(format!("roder-cli-subagents-{}", std::process::id()))
            .join("loads");
        let user_dir = root.join("user");
        let workspace_dir = root.join("workspace");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&user_dir).unwrap();
        std::fs::create_dir_all(&workspace_dir).unwrap();
        std::fs::write(
            user_dir.join("explore.md"),
            r#"---
name: explore
description: Explore the workspace
tools: [echo]
---

Report findings.
"#,
        )
        .unwrap();

        let cfg = roder_config::SubagentsConfig {
            enabled: true,
            default_agent: Some("explore".to_string()),
            disk: roder_config::SubagentsDiskConfig {
                user_dir: Some(user_dir),
                workspace_dir: Some(workspace_dir),
            },
            ..roder_config::SubagentsConfig::default()
        };

        let resolved =
            resolve_subagents_config(Some(&cfg), PROVIDER_MOCK.to_string(), "mock".to_string())
                .await
                .unwrap()
                .unwrap();

        assert!(resolved.enabled);
        assert_eq!(resolved.default_agent, "explore");
        assert_eq!(resolved.definitions.len(), 1);
        assert_eq!(resolved.definitions[0].agent_type, "explore");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn subagents_do_not_default_to_workspace_roder_dir() {
        let cfg = roder_config::SubagentsConfig {
            enabled: true,
            disk: roder_config::SubagentsDiskConfig {
                user_dir: None,
                workspace_dir: None,
            },
            ..roder_config::SubagentsConfig::default()
        };

        assert!(resolve_workspace_agent_dir(&cfg).unwrap().is_none());
    }

    #[tokio::test]
    async fn subagents_disabled_config_skips_loading() {
        let cfg = roder_config::SubagentsConfig {
            enabled: false,
            disk: roder_config::SubagentsDiskConfig {
                user_dir: Some(PathBuf::from("/definitely/not/a/real/agent/dir")),
                workspace_dir: Some(PathBuf::from("/definitely/not/a/real/workspace/dir")),
            },
            ..roder_config::SubagentsConfig::default()
        };

        let resolved =
            resolve_subagents_config(Some(&cfg), PROVIDER_MOCK.to_string(), "mock".to_string())
                .await
                .unwrap()
                .unwrap();

        assert!(!resolved.enabled);
        assert!(resolved.definitions.is_empty());
    }
}
