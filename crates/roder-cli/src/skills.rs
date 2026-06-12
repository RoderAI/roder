use std::sync::Arc;

use roder_api::skills::{SkillActivationState, SkillExposure, SkillSelector, SkillSource};
use roder_app_server::{AppServer, LocalAppClient};
use roder_protocol::{
    JsonRpcRequest, SkillsListResult, SkillsSetEnabledParams, SkillsSetExposureParams,
    SkillsUpdateResult,
};

use crate::{CliOptions, build_runtime_from_config, decode_response};

pub async fn run_skills_cli(args: &[String]) -> anyhow::Result<()> {
    let (runtime, _) = build_runtime_from_config(CliOptions::default()).await?;
    let client = LocalAppClient::new(Arc::new(
        AppServer::new(runtime).with_user_config_persistence(),
    ));
    match args.first().map(String::as_str) {
        Some("list") | None => {
            let result = skills_request::<SkillsListResult>(
                &client,
                "skills/list",
                Some(serde_json::json!({})),
            )
            .await?;
            for skill in result.skills {
                println!(
                    "{}\t{}\t{}\t{}\t{}\t{}",
                    skill.name,
                    source_label(&skill.source),
                    activation_label(skill.activation),
                    exposure_label(skill.exposure),
                    skill.canonical_path,
                    one_line(&skill.description)
                );
                for diagnostic in skill.diagnostics {
                    println!("diagnostic\t{}\t{}", skill.name, one_line(&diagnostic));
                }
            }
            for diagnostic in result.diagnostics {
                println!("diagnostic\tregistry\t{}", one_line(&diagnostic));
            }
        }
        Some("enable") | Some("disable") => {
            let Some(raw_selector) = args.get(1) else {
                anyhow::bail!("usage: roder skills enable|disable <name-or-path>");
            };
            let enabled = args.first().map(String::as_str) == Some("enable");
            let result = skills_request::<SkillsUpdateResult>(
                &client,
                "skills/setEnabled",
                Some(serde_json::to_value(SkillsSetEnabledParams {
                    selector: parse_selector(raw_selector),
                    enabled,
                })?),
            )
            .await?;
            print_matching_or_all(&result, raw_selector);
        }
        Some("exposure") => {
            let Some(raw_selector) = args.get(1) else {
                anyhow::bail!("usage: roder skills exposure <name-or-path> <global|direct-only>");
            };
            let Some(raw_exposure) = args.get(2) else {
                anyhow::bail!("usage: roder skills exposure <name-or-path> <global|direct-only>");
            };
            let result = skills_request::<SkillsUpdateResult>(
                &client,
                "skills/setExposure",
                Some(serde_json::to_value(SkillsSetExposureParams {
                    selector: parse_selector(raw_selector),
                    exposure: parse_exposure(raw_exposure)?,
                })?),
            )
            .await?;
            print_matching_or_all(&result, raw_selector);
        }
        _ => anyhow::bail!(
            "usage: roder skills [list|enable <name-or-path>|disable <name-or-path>|exposure <name-or-path> <global|direct-only>]"
        ),
    }
    Ok(())
}

async fn skills_request<T: serde::de::DeserializeOwned>(
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

fn parse_selector(raw: &str) -> SkillSelector {
    if raw.contains("://") || raw.ends_with("/SKILL.md") {
        SkillSelector::Path {
            path: raw.to_string(),
        }
    } else {
        SkillSelector::Name {
            name: raw.to_string(),
        }
    }
}

fn parse_exposure(raw: &str) -> anyhow::Result<SkillExposure> {
    match raw {
        "global" => Ok(SkillExposure::Global),
        "direct-only" | "direct_only" => Ok(SkillExposure::DirectOnly),
        other => anyhow::bail!("invalid exposure {other:?}; expected global or direct-only"),
    }
}

fn print_matching_or_all(result: &SkillsUpdateResult, raw_selector: &str) {
    let mut printed = false;
    for skill in &result.skills {
        if skill.name == raw_selector || skill.canonical_path == raw_selector {
            printed = true;
            println!(
                "{}\t{}\t{}",
                skill.name,
                activation_label(skill.activation),
                exposure_label(skill.exposure)
            );
        }
    }
    if !printed {
        for skill in &result.skills {
            println!(
                "{}\t{}\t{}",
                skill.name,
                activation_label(skill.activation),
                exposure_label(skill.exposure)
            );
        }
    }
}

fn source_label(source: &SkillSource) -> String {
    match source {
        SkillSource::Workspace => "workspace".to_string(),
        SkillSource::User => "user".to_string(),
        SkillSource::Plugin { plugin_id } => format!("plugin:{plugin_id}"),
        SkillSource::Imported { import_id } => format!("imported:{import_id}"),
        SkillSource::BuiltIn => "built-in".to_string(),
    }
}

fn activation_label(state: SkillActivationState) -> &'static str {
    match state {
        SkillActivationState::Enabled => "enabled",
        SkillActivationState::Disabled => "disabled",
        SkillActivationState::Experimental => "experimental",
    }
}

fn exposure_label(exposure: SkillExposure) -> &'static str {
    match exposure {
        SkillExposure::Global => "global",
        SkillExposure::DirectOnly => "direct-only",
    }
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skills_selector_parses_names_and_paths() {
        assert_eq!(
            parse_selector("commit"),
            SkillSelector::Name {
                name: "commit".to_string()
            }
        );
        assert_eq!(
            parse_selector("roder-builtin://commit/SKILL.md"),
            SkillSelector::Path {
                path: "roder-builtin://commit/SKILL.md".to_string()
            }
        );
    }

    #[test]
    fn skills_exposure_accepts_cli_spellings() {
        assert_eq!(parse_exposure("global").unwrap(), SkillExposure::Global);
        assert_eq!(
            parse_exposure("direct-only").unwrap(),
            SkillExposure::DirectOnly
        );
        assert_eq!(
            parse_exposure("direct_only").unwrap(),
            SkillExposure::DirectOnly
        );
        assert!(parse_exposure("always").is_err());
    }
}
