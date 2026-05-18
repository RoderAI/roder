use std::path::{Path, PathBuf};

use roder_api::distribution::Profile;
use serde_json::json;

use crate::build::{BuildOptions, build_release, offline_from_env};
use crate::catalog::Catalog;
use crate::codegen;
use crate::profile::{BUILT_IN_PROFILES, ProfileExt, built_in_profile};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandResult {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputFormat {
    Text,
    Json,
}

pub fn run(args: &[String], workspace: impl AsRef<Path>) -> CommandResult {
    match run_inner(args, workspace.as_ref()) {
        Ok(result) => result,
        Err(err) => CommandResult {
            status: 1,
            stdout: String::new(),
            stderr: format!("{err}\n"),
        },
    }
}

fn run_inner(args: &[String], workspace: &Path) -> anyhow::Result<CommandResult> {
    let (format, args) = parse_format(args)?;
    if args.is_empty() {
        return catalog_list(workspace, format);
    }
    if args == ["profile", "list"] {
        return profile_list(format);
    }
    if args.len() == 3 && args[0] == "profile" && args[1] == "show" {
        return profile_show(&args[2], format);
    }
    if args == ["catalog", "list"] {
        return catalog_list(workspace, format);
    }
    if args.len() == 3 && args[0] == "catalog" && args[1] == "show" {
        return catalog_show(workspace, &args[2], format);
    }
    if args.len() == 2 && args[0] == "validate" {
        return validate_profile(workspace, &args[1], format);
    }
    if args.first().is_some_and(|arg| arg == "generate") {
        return generate(workspace, &args[1..], format);
    }
    anyhow::bail!("usage: roder-configure [--format json] <generate|validate|profile|catalog>")
}

fn profile_list(format: OutputFormat) -> anyhow::Result<CommandResult> {
    let ids = BUILT_IN_PROFILES
        .iter()
        .map(|profile| profile.id)
        .collect::<Vec<_>>();
    let stdout = match format {
        OutputFormat::Text => format!("{}\n", ids.join("\n")),
        OutputFormat::Json => format!("{}\n", serde_json::to_string(&json!({ "profiles": ids }))?),
    };
    Ok(ok(stdout))
}

fn profile_show(id: &str, format: OutputFormat) -> anyhow::Result<CommandResult> {
    let Some(profile) = BUILT_IN_PROFILES.iter().find(|profile| profile.id == id) else {
        anyhow::bail!("unknown built-in profile `{id}`");
    };
    let stdout = match format {
        OutputFormat::Text => profile.source.to_string(),
        OutputFormat::Json => format!(
            "{}\n",
            serde_json::to_string(&built_in_profile(id)?.unwrap())?
        ),
    };
    Ok(ok(stdout))
}

fn catalog_list(workspace: &Path, format: OutputFormat) -> anyhow::Result<CommandResult> {
    let catalog = Catalog::from_workspace(workspace)?;
    let ids = catalog
        .entries()
        .map(|entry| entry.entry.id.clone())
        .collect::<Vec<_>>();
    let stdout = match format {
        OutputFormat::Text => format!("{}\n", ids.join("\n")),
        OutputFormat::Json => format!(
            "{}\n",
            serde_json::to_string(&json!({ "extensions": ids }))?
        ),
    };
    Ok(ok(stdout))
}

fn catalog_show(workspace: &Path, id: &str, format: OutputFormat) -> anyhow::Result<CommandResult> {
    let catalog = Catalog::from_workspace(workspace)?;
    let entry = catalog
        .get(id)
        .ok_or_else(|| anyhow::anyhow!("unknown extension `{id}`"))?;
    let stdout = match format {
        OutputFormat::Text => format!(
            "{}\n{}\n{}\n",
            entry.entry.id, entry.entry.display_name, entry.entry.description
        ),
        OutputFormat::Json => format!("{}\n", serde_json::to_string(&entry.entry)?),
    };
    Ok(ok(stdout))
}

fn validate_profile(
    workspace: &Path,
    profile_path: &str,
    format: OutputFormat,
) -> anyhow::Result<CommandResult> {
    let catalog = Catalog::from_workspace(workspace)?;
    let profile = Profile::load(profile_path)?;
    match profile.validate(&catalog) {
        Ok(report) => {
            let stdout = match format {
                OutputFormat::Text => "valid\n".to_string(),
                OutputFormat::Json => format!(
                    "{}\n",
                    serde_json::to_string(&json!({
                        "ok": true,
                        "required_env": report.required_env,
                    }))?
                ),
            };
            Ok(ok(stdout))
        }
        Err(err) => {
            let stderr = match format {
                OutputFormat::Text => format!("{err}\n"),
                OutputFormat::Json => format!(
                    "{}\n",
                    serde_json::to_string(&json!({
                        "ok": false,
                        "error": err.to_string(),
                    }))?
                ),
            };
            Ok(CommandResult {
                status: 1,
                stdout: String::new(),
                stderr,
            })
        }
    }
}

fn generate(
    workspace: &Path,
    args: &[String],
    format: OutputFormat,
) -> anyhow::Result<CommandResult> {
    let profile_path = flag_value(args, "--profile")?;
    let out_dir = PathBuf::from(flag_value(args, "--out")?);
    let build = args.iter().any(|arg| arg == "--build");
    let catalog = Catalog::from_workspace(workspace)?;
    let profile = Profile::load(profile_path)?;
    profile.validate(&catalog)?;
    codegen::emit(&profile.manifest, &catalog, &out_dir)?;
    let build_summary = if build {
        let runtime = tokio::runtime::Runtime::new()?;
        Some(
            runtime
                .block_on(build_release(
                    &out_dir,
                    &profile.manifest.name,
                    &profile.manifest.extensions,
                    BuildOptions {
                        offline: offline_from_env(),
                    },
                ))
                .map_err(|err| anyhow::anyhow!("{}", err.first_error_block))?,
        )
    } else {
        None
    };
    let stdout = match format {
        OutputFormat::Text => match build_summary {
            Some(summary) => format!(
                "generated {}\n{}\n{}\n",
                out_dir.display(),
                summary.install_hint,
                summary.extension_summary
            ),
            None => format!("generated {}\n", out_dir.display()),
        },
        OutputFormat::Json => format!(
            "{}\n",
            serde_json::to_string(&json!({
                "ok": true,
                "out": out_dir,
                "install_hint": build_summary.as_ref().map(|summary| summary.install_hint.clone()),
            }))?
        ),
    };
    Ok(ok(stdout))
}

fn parse_format(args: &[String]) -> anyhow::Result<(OutputFormat, Vec<String>)> {
    let mut out = Vec::new();
    let mut format = OutputFormat::Text;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--format" {
            let value = args
                .get(i + 1)
                .ok_or_else(|| anyhow::anyhow!("--format needs a value"))?;
            format = match value.as_str() {
                "json" => OutputFormat::Json,
                "text" => OutputFormat::Text,
                other => anyhow::bail!("unsupported format `{other}`"),
            };
            i += 2;
        } else {
            out.push(args[i].clone());
            i += 1;
        }
    }
    Ok((format, out))
}

fn flag_value<'a>(args: &'a [String], flag: &str) -> anyhow::Result<&'a str> {
    args.windows(2)
        .find_map(|window| (window[0] == flag).then_some(window[1].as_str()))
        .ok_or_else(|| anyhow::anyhow!("{flag} is required"))
}

fn ok(stdout: String) -> CommandResult {
    CommandResult {
        status: 0,
        stdout,
        stderr: String::new(),
    }
}
