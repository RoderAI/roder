use roder_api::extension::ExtensionRegistryBuilder;
use roder_ext_firecrawl_search::{FirecrawlSearchConfig, FirecrawlSearchExtension};
use roder_ext_parallel_search::{ParallelSearchConfig, ParallelSearchExtension};
use roder_ext_perplexity_search::{PerplexitySearchConfig, PerplexitySearchExtension};
use roder_ext_synthetic_search::{SyntheticSearchConfig, SyntheticSearchExtension};
use roder_ext_tavily_search::{TavilySearchConfig, TavilySearchExtension};
use roder_ext_web_search::{
    WebSearchRouterConfig, WebSearchRouterExtension, WebSearchRouterProvider,
};
use roder_web_search::WebSearchProviderKind;

const WEB_SEARCH_DEFAULT_MAX_RESULTS: u8 = 5;
const WEB_SEARCH_MAX_RESULTS_LIMIT: u8 = 20;

#[derive(Debug, Clone, Default)]
pub struct DefaultWebSearchConfig {
    pub enabled: bool,
    pub provider: Option<WebSearchProviderKind>,
    pub firecrawl: DefaultWebSearchProviderConfig,
    pub perplexity: DefaultWebSearchProviderConfig,
    pub tavily: DefaultWebSearchProviderConfig,
    pub parallel: DefaultWebSearchProviderConfig,
    pub synthetic: DefaultWebSearchProviderConfig,
    pub timeout_seconds: Option<u64>,
    pub max_results: Option<u8>,
    pub namespaced_tools: bool,
}

#[derive(Debug, Clone, Default)]
pub struct DefaultWebSearchProviderConfig {
    pub enabled: bool,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub project_id: Option<String>,
    pub search_depth: Option<String>,
    pub mode: Option<String>,
    pub debug_raw_response: bool,
}

pub(crate) fn install_web_search(
    builder: &mut ExtensionRegistryBuilder,
    config: DefaultWebSearchConfig,
) -> anyhow::Result<()> {
    if !config.enabled {
        return Ok(());
    }

    let selected = resolve_selected_web_search_provider(&config)?;
    builder.install(WebSearchRouterExtension::new(WebSearchRouterConfig {
        provider: selected.clone(),
        max_results: config
            .max_results
            .unwrap_or(WEB_SEARCH_DEFAULT_MAX_RESULTS)
            .clamp(1, WEB_SEARCH_MAX_RESULTS_LIMIT),
    }))?;

    // Parallel contributes extract (plus namespaced search) in addition to the
    // canonical web_search router. Install it for the selected Parallel provider
    // even when namespaced_tools is off so parallel_extract is available.
    if let (WebSearchRouterProvider::Parallel(provider_config), false) =
        (&selected, config.namespaced_tools)
    {
        builder.install(ParallelSearchExtension::with_config(provider_config.clone()))?;
    }

    if config.namespaced_tools {
        install_namespaced_web_search_tools(builder, &config)?;
    }

    Ok(())
}

fn resolve_selected_web_search_provider(
    config: &DefaultWebSearchConfig,
) -> anyhow::Result<WebSearchRouterProvider> {
    if let Some(provider) = config.provider {
        return provider_from_config(config, provider).ok_or_else(|| {
            anyhow::anyhow!(
                "web_search provider {:?} is enabled but no API key was configured",
                provider.as_str()
            )
        });
    }

    let providers = [
        WebSearchProviderKind::Firecrawl,
        WebSearchProviderKind::Perplexity,
        WebSearchProviderKind::Tavily,
        WebSearchProviderKind::Parallel,
        WebSearchProviderKind::Synthetic,
    ]
    .into_iter()
    .filter_map(|provider| provider_from_config(config, provider))
    .collect::<Vec<_>>();

    match providers.as_slice() {
        [] => anyhow::bail!("web_search is enabled but no provider API key was configured"),
        [provider] => Ok(provider.clone()),
        _ => anyhow::bail!(
            "web_search.provider must be set when multiple web search providers are configured"
        ),
    }
}

fn install_namespaced_web_search_tools(
    builder: &mut ExtensionRegistryBuilder,
    config: &DefaultWebSearchConfig,
) -> anyhow::Result<()> {
    for kind in [
        WebSearchProviderKind::Firecrawl,
        WebSearchProviderKind::Perplexity,
        WebSearchProviderKind::Tavily,
        WebSearchProviderKind::Parallel,
        WebSearchProviderKind::Synthetic,
    ] {
        match provider_from_config(config, kind) {
            Some(WebSearchRouterProvider::Firecrawl(provider_config)) => {
                builder.install(FirecrawlSearchExtension::with_config(provider_config))?;
            }
            Some(WebSearchRouterProvider::Perplexity(provider_config)) => {
                builder.install(PerplexitySearchExtension::with_config(provider_config))?;
            }
            Some(WebSearchRouterProvider::Tavily(provider_config)) => {
                builder.install(TavilySearchExtension::with_config(provider_config))?;
            }
            Some(WebSearchRouterProvider::Parallel(provider_config)) => {
                builder.install(ParallelSearchExtension::with_config(provider_config))?;
            }
            Some(WebSearchRouterProvider::Synthetic(provider_config)) => {
                builder.install(SyntheticSearchExtension::with_config(provider_config))?;
            }
            None => {}
        }
    }
    Ok(())
}

fn provider_from_config(
    config: &DefaultWebSearchConfig,
    provider: WebSearchProviderKind,
) -> Option<WebSearchRouterProvider> {
    let provider_config = match provider {
        WebSearchProviderKind::Firecrawl => &config.firecrawl,
        WebSearchProviderKind::Perplexity => &config.perplexity,
        WebSearchProviderKind::Tavily => &config.tavily,
        WebSearchProviderKind::Parallel => &config.parallel,
        WebSearchProviderKind::Synthetic => &config.synthetic,
        WebSearchProviderKind::Custom => return None,
    };
    let api_key = provider_config.api_key.as_deref()?.trim();
    if api_key.is_empty() {
        return None;
    }
    let timeout_seconds = config.timeout_seconds.unwrap_or(20);
    match provider {
        WebSearchProviderKind::Firecrawl => {
            let mut cfg = FirecrawlSearchConfig::new(api_key).with_timeout_seconds(timeout_seconds);
            if let Some(base_url) = provider_config.base_url.as_deref() {
                cfg = cfg.with_base_url(base_url);
            }
            cfg = cfg.with_debug_raw_response(provider_config.debug_raw_response);
            Some(WebSearchRouterProvider::Firecrawl(cfg))
        }
        WebSearchProviderKind::Perplexity => {
            let mut cfg =
                PerplexitySearchConfig::new(api_key).with_timeout_seconds(timeout_seconds);
            if let Some(base_url) = provider_config.base_url.as_deref() {
                cfg = cfg.with_base_url(base_url);
            }
            cfg = cfg.with_debug_raw_response(provider_config.debug_raw_response);
            Some(WebSearchRouterProvider::Perplexity(cfg))
        }
        WebSearchProviderKind::Tavily => {
            let mut cfg = TavilySearchConfig::new(api_key).with_timeout_seconds(timeout_seconds);
            if let Some(base_url) = provider_config.base_url.as_deref() {
                cfg = cfg.with_base_url(base_url);
            }
            if let Some(project_id) = provider_config.project_id.as_deref() {
                cfg = cfg.with_project_id(project_id);
            }
            if let Some(search_depth) = provider_config.search_depth.as_deref() {
                cfg = cfg.with_search_depth(search_depth);
            }
            cfg = cfg.with_debug_raw_response(provider_config.debug_raw_response);
            Some(WebSearchRouterProvider::Tavily(cfg))
        }
        WebSearchProviderKind::Parallel => {
            let mut cfg = ParallelSearchConfig::new(api_key).with_timeout_seconds(timeout_seconds);
            if let Some(base_url) = provider_config.base_url.as_deref() {
                cfg = cfg.with_base_url(base_url);
            }
            if let Some(mode) = provider_config.mode.as_deref() {
                cfg = cfg.with_mode(mode);
            }
            cfg = cfg.with_debug_raw_response(provider_config.debug_raw_response);
            Some(WebSearchRouterProvider::Parallel(cfg))
        }
        WebSearchProviderKind::Synthetic => {
            let mut cfg = SyntheticSearchConfig::new(api_key).with_timeout_seconds(timeout_seconds);
            if let Some(base_url) = provider_config.base_url.as_deref() {
                cfg = cfg.with_base_url(base_url);
            }
            cfg = cfg.with_debug_raw_response(provider_config.debug_raw_response);
            Some(WebSearchRouterProvider::Synthetic(cfg))
        }
        WebSearchProviderKind::Custom => None,
    }
}

#[cfg(test)]
mod tests {
    use roder_api::extension::ExtensionRegistry;

    use super::*;
    use crate::{DefaultRegistryConfig, build_default_registry};

    #[test]
    fn default_registry_without_web_search_config_has_no_web_search_tool() {
        let registry = build_default_registry(DefaultRegistryConfig::default()).unwrap();
        let names = contributed_tool_names(&registry).unwrap();

        assert!(!names.contains(&"web_search".to_string()));
    }

    #[test]
    fn default_registry_with_one_web_search_provider_installs_canonical_tool() {
        let registry = build_default_registry(DefaultRegistryConfig {
            web_search: Some(DefaultWebSearchConfig {
                enabled: true,
                provider: Some(WebSearchProviderKind::Tavily),
                tavily: provider_with_key("tavily-key"),
                ..DefaultWebSearchConfig::default()
            }),
            ..DefaultRegistryConfig::default()
        })
        .unwrap();
        let names = contributed_tool_names(&registry).unwrap();

        assert!(names.contains(&"web_search".to_string()));
        assert!(!names.contains(&"tavily_search".to_string()));
    }

    #[test]
    fn selected_web_search_provider_without_key_fails_fast() {
        let err = match build_default_registry(DefaultRegistryConfig {
            web_search: Some(DefaultWebSearchConfig {
                enabled: true,
                provider: Some(WebSearchProviderKind::Tavily),
                ..DefaultWebSearchConfig::default()
            }),
            ..DefaultRegistryConfig::default()
        }) {
            Ok(_) => panic!("selected web_search provider without a key should fail"),
            Err(err) => err,
        };

        let message = err.to_string();
        assert!(message.contains("tavily"));
        assert!(message.contains("API key"));
    }

    #[test]
    fn web_search_namespaced_tools_are_opt_in() {
        let registry = build_default_registry(DefaultRegistryConfig {
            web_search: Some(DefaultWebSearchConfig {
                enabled: true,
                provider: Some(WebSearchProviderKind::Tavily),
                tavily: provider_with_key("tavily-key"),
                namespaced_tools: true,
                ..DefaultWebSearchConfig::default()
            }),
            ..DefaultRegistryConfig::default()
        })
        .unwrap();
        let names = contributed_tool_names(&registry).unwrap();

        assert!(names.contains(&"web_search".to_string()));
        assert!(names.contains(&"tavily_search".to_string()));
    }

    #[test]
    fn parallel_provider_installs_extract_tool_without_namespaced_flag() {
        let registry = build_default_registry(DefaultRegistryConfig {
            web_search: Some(DefaultWebSearchConfig {
                enabled: true,
                provider: Some(WebSearchProviderKind::Parallel),
                parallel: provider_with_key("parallel-key"),
                ..DefaultWebSearchConfig::default()
            }),
            ..DefaultRegistryConfig::default()
        })
        .unwrap();
        let names = contributed_tool_names(&registry).unwrap();

        assert!(names.contains(&"web_search".to_string()));
        assert!(names.contains(&"parallel_search".to_string()));
        assert!(names.contains(&"parallel_extract".to_string()));
    }

    #[test]
    fn default_registry_with_synthetic_search_installs_canonical_tool() {
        let registry = build_default_registry(DefaultRegistryConfig {
            web_search: Some(DefaultWebSearchConfig {
                enabled: true,
                provider: Some(WebSearchProviderKind::Synthetic),
                synthetic: provider_with_key("synthetic-key"),
                ..DefaultWebSearchConfig::default()
            }),
            ..DefaultRegistryConfig::default()
        })
        .unwrap();
        let names = contributed_tool_names(&registry).unwrap();

        assert!(names.contains(&"web_search".to_string()));
        assert!(!names.contains(&"synthetic_search".to_string()));
    }

    #[test]
    fn synthetic_search_namespaced_tools_are_opt_in() {
        let registry = build_default_registry(DefaultRegistryConfig {
            web_search: Some(DefaultWebSearchConfig {
                enabled: true,
                provider: Some(WebSearchProviderKind::Synthetic),
                synthetic: provider_with_key("synthetic-key"),
                namespaced_tools: true,
                ..DefaultWebSearchConfig::default()
            }),
            ..DefaultRegistryConfig::default()
        })
        .unwrap();
        let names = contributed_tool_names(&registry).unwrap();

        assert!(names.contains(&"web_search".to_string()));
        assert!(names.contains(&"synthetic_search".to_string()));
    }

    #[test]
    fn web_search_duplicate_tool_registration_fails() {
        let registry = build_default_registry(DefaultRegistryConfig {
            web_search: Some(DefaultWebSearchConfig {
                enabled: true,
                provider: Some(WebSearchProviderKind::Tavily),
                tavily: provider_with_key("tavily-key"),
                ..DefaultWebSearchConfig::default()
            }),
            ..DefaultRegistryConfig::default()
        })
        .unwrap();
        let contributor = registry
            .tools
            .iter()
            .find(|contributor| contributor.id() == "web-search")
            .unwrap();
        let mut tools = roder_api::tools::ToolRegistry::default();

        contributor.contribute(&mut tools).unwrap();
        let err = contributor.contribute(&mut tools).unwrap_err();

        assert!(err.to_string().contains("web_search"));
        assert!(err.to_string().contains("already registered"));
    }

    #[test]
    fn web_search_tool_specs_do_not_expose_secrets() {
        let secret = "secret-tavily-key";
        let registry = build_default_registry(DefaultRegistryConfig {
            web_search: Some(DefaultWebSearchConfig {
                enabled: true,
                provider: Some(WebSearchProviderKind::Tavily),
                tavily: provider_with_key(secret),
                namespaced_tools: true,
                ..DefaultWebSearchConfig::default()
            }),
            ..DefaultRegistryConfig::default()
        })
        .unwrap();
        let specs = contributed_tool_specs(&registry).unwrap();
        let text = serde_json::to_string(&specs).unwrap();

        assert!(!text.contains(secret));
        assert!(!text.contains("Authorization"));
        assert!(!text.contains("x-api-key"));
    }

    fn provider_with_key(key: &str) -> DefaultWebSearchProviderConfig {
        DefaultWebSearchProviderConfig {
            enabled: true,
            api_key: Some(key.to_string()),
            ..DefaultWebSearchProviderConfig::default()
        }
    }

    fn contributed_tool_names(registry: &ExtensionRegistry) -> anyhow::Result<Vec<String>> {
        Ok(contributed_tool_specs(registry)?
            .into_iter()
            .map(|spec| spec.name)
            .collect())
    }

    fn contributed_tool_specs(
        registry: &ExtensionRegistry,
    ) -> anyhow::Result<Vec<roder_api::tools::ToolSpec>> {
        let mut tools = roder_api::tools::ToolRegistry::default();
        for contributor in &registry.tools {
            contributor.contribute(&mut tools)?;
        }
        Ok(tools.specs())
    }
}
