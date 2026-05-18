use std::path::PathBuf;

use roder_api::marketplace::{
    MarketplaceDescriptor, MarketplaceSource, MarketplaceState, validate_marketplace_id,
};
use roder_config::marketplaces::{
    install_default_marketplaces, load_marketplace_store, read_catalog_from_root,
    refresh_marketplace, resolve_local_source, save_marketplace_store,
};
use roder_extension_host::marketplace::{
    dedupe_plugins, install_plugin_variant, normalize_catalog, preview_plugin_install,
};
use roder_protocol::{
    JsonRpcError, MarketplacePluginParams, MarketplacePluginResult, MarketplacesAddParams,
    MarketplacesAddResult, MarketplacesInstallDefaultParams, MarketplacesInstallDefaultResult,
    MarketplacesListResult, MarketplacesRefreshParams, MarketplacesRefreshResult,
    MarketplacesSearchParams, MarketplacesSearchResult, PluginInstallParams, PluginInstallResult,
    PluginListInstalledResult, PluginPreviewInstallParams, PluginPreviewInstallResult,
    PluginUninstallParams, PluginUninstallResult,
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
        let path = PathBuf::from(&params.local_path);
        if !path.exists() {
            return Err(invalid_params(format!(
                "marketplace path does not exist: {}",
                path.display()
            )));
        }
        let mut store = load_marketplace_store().map_err(internal_error)?;
        let marketplace = MarketplaceDescriptor {
            id: params.id,
            kind: params.kind,
            display_name: params.display_name,
            source: MarketplaceSource::LocalPath {
                path: path.display().to_string(),
            },
            homepage: None,
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

    pub(crate) async fn handle_marketplaces_refresh(
        &self,
        params: MarketplacesRefreshParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let catalog = refresh_marketplace(&params.marketplace_id).map_err(internal_error)?;
        let plugins = normalize_catalog(&catalog).map_err(internal_error)?;
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

    pub(crate) async fn handle_plugins_list_installed(
        &self,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let store = load_marketplace_store().map_err(internal_error)?;
        json_result(PluginListInstalledResult {
            plugins: store.installed_plugins,
        })
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
        let Ok(root) = resolve_local_source(marketplace) else {
            continue;
        };
        let mut catalog = read_catalog_from_root(marketplace.clone(), &root)?;
        if catalog.marketplace.last_refreshed_at.is_none() {
            catalog.marketplace.last_refreshed_at = Some(OffsetDateTime::now_utc());
        }
        entries.extend(normalize_catalog(&catalog)?);
    }
    Ok(entries)
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
