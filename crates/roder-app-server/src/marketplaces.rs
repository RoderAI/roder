use roder_api::marketplace::{
    MarketplaceDescriptor, MarketplaceInstallState, MarketplaceSource, MarketplaceState,
    validate_marketplace_id, validate_marketplace_source, validate_plugin_entry,
};
use roder_config::marketplaces::{
    infer_kind_from_root, infer_kind_from_source, install_default_marketplaces,
    load_marketplace_store, read_catalog_from_root, refresh_marketplace,
    resolve_marketplace_source, resolve_source, save_marketplace_store,
};
use roder_extension_host::marketplace::{
    dedupe_plugins, install_plugin_variant, normalize_catalog, preview_plugin_install,
};
use roder_protocol::{
    JsonRpcError, MarketplacePluginParams, MarketplacePluginResult, MarketplacesAddParams,
    MarketplacesAddResult, MarketplacesInstallDefaultParams, MarketplacesInstallDefaultResult,
    MarketplacesListResult, MarketplacesRefreshParams, MarketplacesRefreshResult,
    MarketplacesRemoveParams, MarketplacesRemoveResult, MarketplacesSearchParams,
    MarketplacesSearchResult, PluginDisableParams, PluginDisableResult,
    PluginInstallAllVariantsParams, PluginInstallAllVariantsResult, PluginInstallParams,
    PluginInstallResult, PluginListInstalledResult, PluginPreviewInstallParams,
    PluginPreviewInstallResult, PluginUninstallParams, PluginUninstallResult,
};
use time::OffsetDateTime;

use crate::server::AppServer;

impl AppServer {
    pub(crate) async fn handle_marketplaces_list(&self) -> Result<serde_json::Value, JsonRpcError> {
        let store = load_marketplace_store().map_err(internal_error)?;
        json_result(MarketplacesListResult {
            marketplaces: store.marketplaces,
        })
    }

    pub(crate) async fn handle_marketplaces_install_default(
        &self,
        params: MarketplacesInstallDefaultParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let marketplaces =
            install_default_marketplaces(params.selection).map_err(internal_error)?;
        json_result(MarketplacesInstallDefaultResult { marketplaces })
    }

    pub(crate) async fn handle_marketplaces_add(
        &self,
        params: MarketplacesAddParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        validate_marketplace_id(&params.id).map_err(invalid_params)?;
        validate_marketplace_source(&params.source).map_err(invalid_params)?;
        if let MarketplaceSource::LocalPath { path } = &params.source {
            let path = std::path::PathBuf::from(path);
            if !path.exists() {
                return Err(invalid_params(format!(
                    "marketplace path does not exist: {}",
                    path.display()
                )));
            }
        }
        let mut store = load_marketplace_store().map_err(internal_error)?;
        if store
            .marketplaces
            .iter()
            .any(|marketplace| marketplace.id == params.id)
        {
            return Err(invalid_params(
                roder_api::marketplace::MarketplaceError::DuplicateMarketplace { id: params.id },
            ));
        }
        let kind = params.kind.unwrap_or_else(|| {
            resolve_source(&params.id, &params.source)
                .map(|root| infer_kind_from_root(&root))
                .unwrap_or_else(|_| infer_kind_from_source(&params.source))
        });
        let marketplace = MarketplaceDescriptor {
            id: params.id,
            kind,
            display_name: params.display_name,
            homepage: homepage_for_source(&params.source),
            source: params.source,
            owner_name: None,
            owner_email: None,
            description: None,
            is_default: false,
            enabled: true,
            state: MarketplaceState::Installed,
            last_refreshed_at: None,
            content_hash: None,
        };
        store.upsert_marketplace(marketplace.clone());
        save_marketplace_store(&store).map_err(internal_error)?;
        json_result(MarketplacesAddResult { marketplace })
    }

    pub(crate) async fn handle_marketplaces_remove(
        &self,
        params: MarketplacesRemoveParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let mut store = load_marketplace_store().map_err(internal_error)?;
        let Some(existing) = store
            .marketplaces
            .iter()
            .find(|marketplace| marketplace.id == params.marketplace_id)
            .cloned()
        else {
            return json_result(MarketplacesRemoveResult { removed: false });
        };
        let removed = if existing.is_default {
            let mut marketplace = existing;
            marketplace.enabled = false;
            marketplace.state = MarketplaceState::RemovedByUser;
            store.upsert_marketplace(marketplace);
            true
        } else {
            store.remove_marketplace(&params.marketplace_id)
        };
        save_marketplace_store(&store).map_err(internal_error)?;
        json_result(MarketplacesRemoveResult { removed })
    }

    pub(crate) async fn handle_marketplaces_refresh(
        &self,
        params: MarketplacesRefreshParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let catalog = refresh_marketplace(&params.marketplace_id).map_err(internal_error)?;
        let plugins = normalize_catalog(&catalog).map_err(internal_error)?;
        validate_marketplace_entries(&catalog.marketplace.id, &plugins).map_err(internal_error)?;
        json_result(MarketplacesRefreshResult {
            marketplace: catalog.marketplace,
            plugins,
        })
    }

    pub(crate) async fn handle_marketplaces_search(
        &self,
        params: MarketplacesSearchParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let store = load_marketplace_store().map_err(internal_error)?;
        let query = params.query.unwrap_or_default().to_ascii_lowercase();
        let installed = store
            .installed_plugins
            .iter()
            .map(|plugin| plugin.variant_key.clone())
            .collect::<Vec<_>>();
        let mut entries =
            collect_marketplace_entries(&store.marketplaces).map_err(internal_error)?;
        if !query.trim().is_empty() {
            entries.retain(|entry| {
                entry.plugin_id.to_ascii_lowercase().contains(&query)
                    || entry.display_name.to_ascii_lowercase().contains(&query)
                    || entry
                        .description
                        .as_deref()
                        .unwrap_or_default()
                        .to_ascii_lowercase()
                        .contains(&query)
                    || entry
                        .tags
                        .iter()
                        .any(|tag| tag.to_ascii_lowercase().contains(&query))
            });
        }
        json_result(MarketplacesSearchResult {
            plugins: dedupe_plugins(&entries, &installed),
        })
    }

    pub(crate) async fn handle_marketplace_plugin(
        &self,
        params: MarketplacePluginParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let plugin = find_marketplace_entry(&params.marketplace_id, &params.plugin_id)
            .map_err(internal_error)?;
        json_result(MarketplacePluginResult { plugin })
    }

    pub(crate) async fn handle_plugins_preview_install(
        &self,
        params: PluginPreviewInstallParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let entry = find_marketplace_entry(&params.marketplace_id, &params.plugin_id)
            .map_err(internal_error)?
            .ok_or_else(|| not_found("plugin not found"))?;
        json_result(PluginPreviewInstallResult {
            preview: preview_plugin_install(&entry),
        })
    }

    pub(crate) async fn handle_plugins_install(
        &self,
        params: PluginInstallParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let entry = find_marketplace_entry(&params.marketplace_id, &params.plugin_id)
            .map_err(internal_error)?
            .ok_or_else(|| not_found("plugin not found"))?;
        let record = install_plugin_variant(&entry).map_err(internal_error)?;
        let mut store = load_marketplace_store().map_err(internal_error)?;
        store.upsert_installed_plugin(record.clone());
        save_marketplace_store(&store).map_err(internal_error)?;
        json_result(PluginInstallResult { plugin: record })
    }

    pub(crate) async fn handle_plugins_install_all_variants(
        &self,
        params: PluginInstallAllVariantsParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let store = load_marketplace_store().map_err(internal_error)?;
        let entries = collect_marketplace_entries(&store.marketplaces).map_err(internal_error)?;
        let seed = entries
            .iter()
            .find(|entry| {
                entry.marketplace_id == params.marketplace_id && entry.plugin_id == params.plugin_id
            })
            .ok_or_else(|| not_found("plugin not found"))?;
        let installed_keys = store
            .installed_plugins
            .iter()
            .map(|plugin| plugin.variant_key.clone())
            .collect::<Vec<_>>();
        let group = dedupe_plugins(&entries, &installed_keys)
            .into_iter()
            .find(|plugin| {
                plugin.variants.iter().any(|variant| {
                    variant.marketplace_id == seed.marketplace_id
                        && variant.plugin_id == seed.plugin_id
                })
            })
            .ok_or_else(|| not_found("plugin variant group not found"))?;
        let mut store = store;
        let mut installed = Vec::new();
        for variant in group.variants {
            let Some(entry) = entries.iter().find(|entry| {
                entry.marketplace_id == variant.marketplace_id
                    && entry.plugin_id == variant.plugin_id
            }) else {
                continue;
            };
            let record = install_plugin_variant(entry).map_err(internal_error)?;
            store.upsert_installed_plugin(record.clone());
            installed.push(record);
        }
        save_marketplace_store(&store).map_err(internal_error)?;
        json_result(PluginInstallAllVariantsResult { plugins: installed })
    }

    pub(crate) async fn handle_plugins_list_installed(
        &self,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let store = load_marketplace_store().map_err(internal_error)?;
        json_result(PluginListInstalledResult {
            plugins: store.installed_plugins,
        })
    }

    pub(crate) async fn handle_plugins_disable(
        &self,
        params: PluginDisableParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let mut store = load_marketplace_store().map_err(internal_error)?;
        let mut disabled = None;
        for plugin in &mut store.installed_plugins {
            if plugin.variant_key == params.variant_key {
                plugin.state = MarketplaceInstallState::Disabled;
                disabled = Some(plugin.clone());
                break;
            }
        }
        save_marketplace_store(&store).map_err(internal_error)?;
        json_result(PluginDisableResult { plugin: disabled })
    }

    pub(crate) async fn handle_plugins_uninstall(
        &self,
        params: PluginUninstallParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let mut store = load_marketplace_store().map_err(internal_error)?;
        let before = store.installed_plugins.len();
        store
            .installed_plugins
            .retain(|plugin| plugin.variant_key != params.variant_key);
        let removed = before != store.installed_plugins.len();
        save_marketplace_store(&store).map_err(internal_error)?;
        json_result(PluginUninstallResult { removed })
    }
}

fn collect_marketplace_entries(
    marketplaces: &[MarketplaceDescriptor],
) -> anyhow::Result<Vec<roder_api::marketplace::MarketplacePluginEntry>> {
    let mut entries = Vec::new();
    for marketplace in marketplaces
        .iter()
        .filter(|marketplace| marketplace.enabled && marketplace.state != MarketplaceState::BakedIn)
    {
        let Ok(root) = resolve_marketplace_source(marketplace) else {
            continue;
        };
        let mut catalog = read_catalog_from_root(marketplace.clone(), &root)?;
        if catalog.marketplace.last_refreshed_at.is_none() {
            catalog.marketplace.last_refreshed_at = Some(OffsetDateTime::now_utc());
        }
        let normalized = normalize_catalog(&catalog)?;
        validate_marketplace_entries(&marketplace.id, &normalized)?;
        entries.extend(normalized);
    }
    Ok(entries)
}

fn validate_marketplace_entries(
    marketplace_id: &str,
    entries: &[roder_api::marketplace::MarketplacePluginEntry],
) -> anyhow::Result<()> {
    let mut seen = std::collections::BTreeSet::new();
    for entry in entries {
        validate_plugin_entry(entry)?;
        if !seen.insert(entry.plugin_id.clone()) {
            return Err(roder_api::marketplace::MarketplaceError::DuplicatePlugin {
                marketplace_id: marketplace_id.to_string(),
                plugin_id: entry.plugin_id.clone(),
            }
            .into());
        }
    }
    Ok(())
}

fn find_marketplace_entry(
    marketplace_id: &str,
    plugin_id: &str,
) -> anyhow::Result<Option<roder_api::marketplace::MarketplacePluginEntry>> {
    let store = load_marketplace_store()?;
    let entries = collect_marketplace_entries(&store.marketplaces)?;
    Ok(entries
        .into_iter()
        .find(|entry| entry.marketplace_id == marketplace_id && entry.plugin_id == plugin_id))
}

fn invalid_params(err: impl std::fmt::Display) -> JsonRpcError {
    JsonRpcError {
        code: -32602,
        message: format!("Invalid params: {err}"),
        data: None,
    }
}

fn internal_error(err: impl std::fmt::Display) -> JsonRpcError {
    let details = format!("{err:#}");
    JsonRpcError {
        code: -32000,
        message: details.clone(),
        data: Some(serde_json::json!({ "details": details })),
    }
}

fn not_found(message: impl Into<String>) -> JsonRpcError {
    JsonRpcError {
        code: -32004,
        message: message.into(),
        data: None,
    }
}

fn json_result<T: serde::Serialize>(value: T) -> Result<serde_json::Value, JsonRpcError> {
    serde_json::to_value(value).map_err(internal_error)
}

fn homepage_for_source(source: &MarketplaceSource) -> Option<String> {
    match source {
        MarketplaceSource::Github { repo, .. } => Some(format!("https://github.com/{repo}")),
        MarketplaceSource::Git { url, .. } | MarketplaceSource::HttpJson { url } => {
            Some(url.clone())
        }
        MarketplaceSource::LocalPath { .. } => None,
    }
}
