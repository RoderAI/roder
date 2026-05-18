use std::sync::Arc;

use roder_api::marketplace::{DefaultMarketplaceSelection, MarketplaceKind};
use roder_app_server::{AppServer, LocalAppClient};
use roder_protocol::{
    JsonRpcRequest, MarketplacePluginParams, MarketplacePluginResult, MarketplacesAddParams,
    MarketplacesAddResult, MarketplacesInstallDefaultParams, MarketplacesInstallDefaultResult,
    MarketplacesListResult, MarketplacesRefreshParams, MarketplacesRefreshResult,
    MarketplacesSearchParams, MarketplacesSearchResult, PluginInstallParams, PluginInstallResult,
    PluginListInstalledResult, PluginPreviewInstallParams, PluginPreviewInstallResult,
    PluginUninstallParams, PluginUninstallResult,
};

use crate::{CliOptions, build_runtime_from_config, decode_response};

pub async fn run_setup_cli(args: &[String]) -> anyhow::Result<()> {
    match args.first().map(String::as_str) {
        Some("marketplaces") => {
            let selection = args
                .iter()
                .position(|arg| arg == "--defaults")
                .and_then(|idx| args.get(idx + 1))
                .map(|raw| raw.parse())
                .transpose()?
                .unwrap_or(DefaultMarketplaceSelection::All);
            install_default_marketplaces_cli(selection).await
        }
        _ => anyhow::bail!(
            "usage: roder setup marketplaces [--defaults all|anthropic|cursor|codex|none]"
        ),
    }
}

pub async fn run_marketplace_cli(args: &[String]) -> anyhow::Result<()> {
    let (runtime, _) = build_runtime_from_config(CliOptions::default()).await?;
    let client = LocalAppClient::new(Arc::new(AppServer::new(runtime)));
    match args.first().map(String::as_str) {
        Some("list") => {
            let result =
                marketplace_request::<MarketplacesListResult>(&client, "marketplaces/list", None)
                    .await?;
            for marketplace in result.marketplaces {
                println!(
                    "{}\t{:?}\t{:?}\t{}\t{}",
                    marketplace.id,
                    marketplace.kind,
                    marketplace.state,
                    if marketplace.enabled {
                        "enabled"
                    } else {
                        "disabled"
                    },
                    marketplace.display_name
                );
            }
        }
        Some("install-default") | Some("install-defaults") => {
            let selection = args
                .get(1)
                .map(|raw| raw.parse())
                .transpose()?
                .unwrap_or(DefaultMarketplaceSelection::All);
            let result = marketplace_request::<MarketplacesInstallDefaultResult>(
                &client,
                "marketplaces/install_default",
                Some(serde_json::to_value(MarketplacesInstallDefaultParams {
                    selection,
                })?),
            )
            .await?;
            for marketplace in result.marketplaces {
                println!(
                    "{}\t{:?}\t{}",
                    marketplace.id, marketplace.state, marketplace.display_name
                );
            }
        }
        Some("add") => {
            let Some(id) = args.get(1) else {
                anyhow::bail!(
                    "usage: roder marketplace add ID --kind claude|cursor|codex|custom --path PATH [--name NAME]"
                );
            };
            let kind = marketplace_kind_arg(args)?;
            let path = flag_value(args, "--path")
                .ok_or_else(|| anyhow::anyhow!("roder marketplace add requires --path PATH"))?;
            let display_name = flag_value(args, "--name").unwrap_or_else(|| id.clone());
            let result = marketplace_request::<MarketplacesAddResult>(
                &client,
                "marketplaces/add",
                Some(serde_json::to_value(MarketplacesAddParams {
                    id: id.clone(),
                    kind,
                    display_name,
                    local_path: path,
                })?),
            )
            .await?;
            println!(
                "added\t{}\t{:?}\t{}",
                result.marketplace.id, result.marketplace.kind, result.marketplace.display_name
            );
        }
        Some("refresh") => {
            let Some(marketplace_id) = args.get(1) else {
                anyhow::bail!("usage: roder marketplace refresh MARKETPLACE_ID");
            };
            let result = marketplace_request::<MarketplacesRefreshResult>(
                &client,
                "marketplaces/refresh",
                Some(serde_json::to_value(MarketplacesRefreshParams {
                    marketplace_id: marketplace_id.clone(),
                })?),
            )
            .await?;
            println!(
                "refreshed\t{}\t{} plugins",
                result.marketplace.id,
                result.plugins.len()
            );
            print_marketplace_plugins(&result.plugins);
        }
        Some("search") => {
            let result = marketplace_request::<MarketplacesSearchResult>(
                &client,
                "marketplaces/search",
                Some(serde_json::to_value(MarketplacesSearchParams {
                    query: args.get(1).cloned(),
                })?),
            )
            .await?;
            for plugin in result.plugins {
                println!(
                    "{}\t{}\t{} variants",
                    plugin.identity_key.canonical_slug,
                    plugin.display_name,
                    plugin.variants.len()
                );
            }
        }
        Some("show") => {
            let (marketplace_id, plugin_id) = marketplace_plugin_args(args)?;
            let result = marketplace_request::<MarketplacePluginResult>(
                &client,
                "marketplaces/plugin",
                Some(serde_json::to_value(MarketplacePluginParams {
                    marketplace_id,
                    plugin_id,
                })?),
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&result.plugin)?);
        }
        _ => anyhow::bail!(
            "usage: roder marketplace <list|install-default [selection]|add|refresh|search|show>"
        ),
    }
    Ok(())
}

pub async fn run_plugin_cli(args: &[String]) -> anyhow::Result<()> {
    let (runtime, _) = build_runtime_from_config(CliOptions::default()).await?;
    let client = LocalAppClient::new(Arc::new(AppServer::new(runtime)));
    match args.first().map(String::as_str) {
        Some("preview") => {
            let (marketplace_id, plugin_id) = marketplace_plugin_args(args)?;
            let result = marketplace_request::<PluginPreviewInstallResult>(
                &client,
                "plugins/preview_install",
                Some(serde_json::to_value(PluginPreviewInstallParams {
                    marketplace_id,
                    plugin_id,
                })?),
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&result.preview)?);
        }
        Some("install") => {
            let (marketplace_id, plugin_id) = marketplace_plugin_args(args)?;
            let result = marketplace_request::<PluginInstallResult>(
                &client,
                "plugins/install",
                Some(serde_json::to_value(PluginInstallParams {
                    marketplace_id,
                    plugin_id,
                })?),
            )
            .await?;
            println!(
                "installed\t{}\t{}",
                result.plugin.variant_key, result.plugin.install_path
            );
        }
        Some("list") => {
            let result = marketplace_request::<PluginListInstalledResult>(
                &client,
                "plugins/list_installed",
                None,
            )
            .await?;
            for plugin in result.plugins {
                println!(
                    "{}\t{}\t{}\t{:?}",
                    plugin.variant_key, plugin.marketplace_id, plugin.plugin_id, plugin.state
                );
            }
        }
        Some("uninstall") => {
            let Some(variant_key) = args.get(1) else {
                anyhow::bail!("usage: roder plugin uninstall VARIANT_KEY");
            };
            let result = marketplace_request::<PluginUninstallResult>(
                &client,
                "plugins/uninstall",
                Some(serde_json::to_value(PluginUninstallParams {
                    variant_key: variant_key.clone(),
                })?),
            )
            .await?;
            println!("removed\t{}", result.removed);
        }
        _ => anyhow::bail!(
            "usage: roder plugin <preview|install MARKETPLACE_ID PLUGIN_ID|list|uninstall VARIANT_KEY>"
        ),
    }
    Ok(())
}

async fn install_default_marketplaces_cli(
    selection: DefaultMarketplaceSelection,
) -> anyhow::Result<()> {
    let (runtime, _) = build_runtime_from_config(CliOptions::default()).await?;
    let client = LocalAppClient::new(Arc::new(AppServer::new(runtime)));
    let result = marketplace_request::<MarketplacesInstallDefaultResult>(
        &client,
        "marketplaces/install_default",
        Some(serde_json::to_value(MarketplacesInstallDefaultParams {
            selection,
        })?),
    )
    .await?;
    for marketplace in result.marketplaces {
        println!(
            "{}\t{:?}\t{}",
            marketplace.id, marketplace.state, marketplace.display_name
        );
    }
    Ok(())
}

async fn marketplace_request<T: serde::de::DeserializeOwned>(
    client: &LocalAppClient,
    method: &str,
    params: Option<serde_json::Value>,
) -> anyhow::Result<T> {
    let res = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(method)),
            method: method.to_string(),
            params,
        })
        .await;
    decode_response(res)
}

fn marketplace_plugin_args(args: &[String]) -> anyhow::Result<(String, String)> {
    let Some(marketplace_id) = args.get(1) else {
        anyhow::bail!("expected MARKETPLACE_ID PLUGIN_ID");
    };
    let Some(plugin_id) = args.get(2) else {
        anyhow::bail!("expected MARKETPLACE_ID PLUGIN_ID");
    };
    Ok((marketplace_id.clone(), plugin_id.clone()))
}

fn marketplace_kind_arg(args: &[String]) -> anyhow::Result<MarketplaceKind> {
    let Some(raw) = flag_value(args, "--kind") else {
        anyhow::bail!("roder marketplace add requires --kind claude|cursor|codex|roder|custom");
    };
    match raw.as_str() {
        "claude" | "anthropic" => Ok(MarketplaceKind::Claude),
        "cursor" => Ok(MarketplaceKind::Cursor),
        "codex" => Ok(MarketplaceKind::Codex),
        "roder" => Ok(MarketplaceKind::Roder),
        "custom" => Ok(MarketplaceKind::Custom),
        value => anyhow::bail!("unknown marketplace kind {value}"),
    }
}

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|idx| args.get(idx + 1))
        .cloned()
}

fn print_marketplace_plugins(plugins: &[roder_api::marketplace::MarketplacePluginEntry]) {
    for plugin in plugins {
        println!(
            "{}\t{:?}\t{}\t{}",
            plugin.plugin_id, plugin.kind, plugin.display_name, plugin.identity_key.canonical_slug
        );
    }
}
