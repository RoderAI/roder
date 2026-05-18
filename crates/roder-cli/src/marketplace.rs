use std::sync::Arc;

use roder_api::marketplace::{DefaultMarketplaceSelection, MarketplaceKind, MarketplaceSource};
use roder_app_server::{AppServer, LocalAppClient};
use roder_protocol::{
    JsonRpcRequest, MarketplacePluginParams, MarketplacePluginResult, MarketplacesAddParams,
    MarketplacesAddResult, MarketplacesInstallDefaultParams, MarketplacesInstallDefaultResult,
    MarketplacesListResult, MarketplacesRefreshParams, MarketplacesRefreshResult,
    MarketplacesRemoveParams, MarketplacesRemoveResult, MarketplacesSearchParams,
    MarketplacesSearchResult, PluginDisableParams, PluginDisableResult,
    PluginInstallAllVariantsParams, PluginInstallAllVariantsResult, PluginInstallParams,
    PluginInstallResult, PluginListInstalledResult, PluginPreviewInstallParams,
    PluginPreviewInstallResult, PluginUninstallParams, PluginUninstallResult,
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
                    "usage: roder marketplace add ID [--kind auto|claude|cursor|codex|roder|custom] (--path PATH|--github OWNER/REPO|--git URL|--http-json URL) [--name NAME] [--ref REF] [--catalog-path PATH] [--plugin-root PATH]"
                );
            };
            let kind = marketplace_kind_arg(args)?;
            let source = marketplace_source_arg(args)?;
            let display_name = flag_value(args, "--name").unwrap_or_else(|| id.clone());
            let result = marketplace_request::<MarketplacesAddResult>(
                &client,
                "marketplaces/add",
                Some(serde_json::to_value(MarketplacesAddParams {
                    id: id.clone(),
                    kind,
                    display_name,
                    source,
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
        Some("remove") => {
            let Some(marketplace_id) = args.get(1) else {
                anyhow::bail!("usage: roder marketplace remove MARKETPLACE_ID");
            };
            let result = marketplace_request::<MarketplacesRemoveResult>(
                &client,
                "marketplaces/remove",
                Some(serde_json::to_value(MarketplacesRemoveParams {
                    marketplace_id: marketplace_id.clone(),
                })?),
            )
            .await?;
            println!("removed\t{}", result.removed);
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
            "usage: roder marketplace <list|install-default [selection]|add|refresh|remove|search|show>"
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
            if args.iter().any(|arg| arg == "--all-variants") {
                let result = marketplace_request::<PluginInstallAllVariantsResult>(
                    &client,
                    "plugins/install_all_variants",
                    Some(serde_json::to_value(PluginInstallAllVariantsParams {
                        marketplace_id,
                        plugin_id,
                    })?),
                )
                .await?;
                for plugin in result.plugins {
                    println!("installed\t{}\t{}", plugin.variant_key, plugin.install_path);
                }
            } else {
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
        }
        Some("install-all") | Some("install-all-variants") => {
            let (marketplace_id, plugin_id) = marketplace_plugin_args(args)?;
            let result = marketplace_request::<PluginInstallAllVariantsResult>(
                &client,
                "plugins/install_all_variants",
                Some(serde_json::to_value(PluginInstallAllVariantsParams {
                    marketplace_id,
                    plugin_id,
                })?),
            )
            .await?;
            for plugin in result.plugins {
                println!("installed\t{}\t{}", plugin.variant_key, plugin.install_path);
            }
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
        Some("disable") => {
            let Some(variant_key) = args.get(1) else {
                anyhow::bail!("usage: roder plugin disable VARIANT_KEY");
            };
            let result = marketplace_request::<PluginDisableResult>(
                &client,
                "plugins/disable",
                Some(serde_json::to_value(PluginDisableParams {
                    variant_key: variant_key.clone(),
                })?),
            )
            .await?;
            println!("disabled\t{}", result.plugin.is_some());
        }
        _ => anyhow::bail!(
            "usage: roder plugin <preview|install [--all-variants]|install-all|list|disable|uninstall>"
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

fn marketplace_kind_arg(args: &[String]) -> anyhow::Result<Option<MarketplaceKind>> {
    let Some(raw) = flag_value(args, "--kind") else {
        return Ok(None);
    };
    match raw.as_str() {
        "auto" => Ok(None),
        "claude" | "anthropic" => Ok(Some(MarketplaceKind::Claude)),
        "cursor" => Ok(Some(MarketplaceKind::Cursor)),
        "codex" => Ok(Some(MarketplaceKind::Codex)),
        "roder" => Ok(Some(MarketplaceKind::Roder)),
        "custom" => Ok(Some(MarketplaceKind::Custom)),
        value => anyhow::bail!("unknown marketplace kind {value}"),
    }
}

fn marketplace_source_arg(args: &[String]) -> anyhow::Result<MarketplaceSource> {
    let ref_name = flag_value(args, "--ref");
    let catalog_path = flag_value(args, "--catalog-path");
    if let Some(path) = flag_value(args, "--path") {
        return Ok(MarketplaceSource::LocalPath { path });
    }
    if let Some(repo) = flag_value(args, "--github") {
        return Ok(MarketplaceSource::Github {
            repo,
            ref_name,
            catalog_path,
            plugin_root: flag_value(args, "--plugin-root"),
        });
    }
    if let Some(url) = flag_value(args, "--git") {
        return Ok(MarketplaceSource::Git {
            url,
            ref_name,
            catalog_path,
        });
    }
    if let Some(url) = flag_value(args, "--http-json") {
        return Ok(MarketplaceSource::HttpJson { url });
    }
    if let Some(raw) = args.get(2) {
        if raw.starts_with("http://") || raw.starts_with("https://") {
            if raw.ends_with(".git") {
                return Ok(MarketplaceSource::Git {
                    url: raw.clone(),
                    ref_name,
                    catalog_path,
                });
            }
            return Ok(MarketplaceSource::HttpJson { url: raw.clone() });
        }
        if raw.contains('/') && !std::path::Path::new(raw).exists() {
            return Ok(MarketplaceSource::Github {
                repo: raw.clone(),
                ref_name,
                catalog_path,
                plugin_root: flag_value(args, "--plugin-root"),
            });
        }
        return Ok(MarketplaceSource::LocalPath { path: raw.clone() });
    }
    anyhow::bail!(
        "roder marketplace add requires --path PATH, --github OWNER/REPO, --git URL, or --http-json URL"
    )
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
