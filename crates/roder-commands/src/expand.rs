use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use roder_api::{
    context::{ContextBlock, ContextBlockKind},
    policy_mode::{PolicyMode, PolicyModeConfig},
    skills::SkillSelector,
};
use roder_skills::{SkillRegistry, SkillResolutionError, render_skill_body};
use serde_json::json;

use crate::{
    spec::{CommandSpec, FileInclude, ShellInclude, UrlInclude},
    template::{
        TemplateContext, default_include_id, include_reference, include_template_key,
        render_template,
    },
};

pub trait ShellRunner {
    fn run_shell(&self, command: &str, timeout_seconds: u64) -> Result<String>;
}

pub trait UrlFetcher {
    fn fetch_url(&self, url: &str, timeout_seconds: u64, max_bytes: usize) -> Result<String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandExpansionOptions {
    pub allow_shell_includes: bool,
    pub allow_url_includes: bool,
    pub allowed_url_hosts: Vec<String>,
    pub include_timeout_seconds: u64,
    pub max_include_bytes: usize,
    pub policy_mode: PolicyMode,
}

impl Default for CommandExpansionOptions {
    fn default() -> Self {
        Self {
            allow_shell_includes: false,
            allow_url_includes: false,
            allowed_url_hosts: Vec::new(),
            include_timeout_seconds: 5,
            max_include_bytes: 65_536,
            policy_mode: PolicyMode::Default,
        }
    }
}

pub struct CommandExpansionRequest<'a> {
    pub spec: &'a CommandSpec,
    pub arguments: &'a str,
    pub workspace_root: &'a Path,
    pub options: CommandExpansionOptions,
    pub shell_runner: Option<&'a dyn ShellRunner>,
    pub url_fetcher: Option<&'a dyn UrlFetcher>,
    pub skill_registry: Option<&'a SkillRegistry>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CommandExpansion {
    pub command_name: String,
    pub message: String,
    pub context_blocks: Vec<ContextBlock>,
    pub allowed_tools: Vec<String>,
    pub model: Option<String>,
    pub agent: Option<String>,
}

pub fn expand_command(request: CommandExpansionRequest<'_>) -> Result<CommandExpansion> {
    let mut resolved = resolve_includes(&request)?;
    resolve_feature_skill_bindings(&request, &mut resolved)?;
    let template_context = TemplateContext {
        arguments: request.arguments.trim().to_string(),
        includes: resolved.references,
    };
    let message = render_template(&request.spec.body, &template_context)?;

    Ok(CommandExpansion {
        command_name: request.spec.name.clone(),
        message,
        context_blocks: resolved.blocks,
        allowed_tools: request.spec.allowed_tools.clone(),
        model: request.spec.model.clone(),
        agent: request.spec.agent.clone(),
    })
}

#[derive(Debug, Default)]
struct ResolvedIncludes {
    references: BTreeMap<String, String>,
    blocks: Vec<ContextBlock>,
}

fn resolve_includes(request: &CommandExpansionRequest<'_>) -> Result<ResolvedIncludes> {
    let mut resolved = ResolvedIncludes::default();
    let root = canonical_workspace_root(request.workspace_root)?;

    for (index, include) in request.spec.include.files.iter().enumerate() {
        resolve_file_include(request, &root, index, include, &mut resolved)?;
    }
    for (index, include) in request.spec.include.shell.iter().enumerate() {
        resolve_shell_include(request, index, include, &mut resolved)?;
    }
    for (index, include) in request.spec.include.urls.iter().enumerate() {
        resolve_url_include(request, index, include, &mut resolved)?;
    }

    Ok(resolved)
}

fn resolve_feature_skill_bindings(
    request: &CommandExpansionRequest<'_>,
    resolved: &mut ResolvedIncludes,
) -> Result<()> {
    if request.spec.feature_skill_bindings.is_empty() {
        return Ok(());
    }
    let registry = request.skill_registry.ok_or_else(|| {
        anyhow::anyhow!(
            "command `{}` requires skill bindings but no skill registry is configured",
            request.spec.name
        )
    })?;
    for binding in &request.spec.feature_skill_bindings {
        match registry.resolve(&binding.skill_selector) {
            Ok(skill) => resolved.blocks.push(render_skill_body(skill)),
            Err(err) if binding.required => {
                bail!(
                    "command `{}` required skill {} could not be activated: {}",
                    request.spec.name,
                    selector_label(&binding.skill_selector),
                    skill_resolution_error_message(&err)
                );
            }
            Err(_) => {}
        }
    }
    Ok(())
}

fn selector_label(selector: &SkillSelector) -> String {
    match selector {
        SkillSelector::Name { name } => name.clone(),
        SkillSelector::Path { path } => path.clone(),
    }
}

fn skill_resolution_error_message(error: &SkillResolutionError) -> String {
    match error {
        SkillResolutionError::Missing(_) => "skill not found".to_string(),
        SkillResolutionError::Disabled(path) => format!("skill disabled: {path}"),
        SkillResolutionError::Ambiguous {
            name,
            canonical_paths,
        } => format!(
            "skill name {name} is ambiguous; select by canonical path: {}",
            canonical_paths.join(", ")
        ),
    }
}

fn canonical_workspace_root(root: &Path) -> Result<PathBuf> {
    fs::canonicalize(root)
        .with_context(|| format!("canonicalize workspace root {}", root.display()))
}

fn resolve_file_include(
    request: &CommandExpansionRequest<'_>,
    root: &Path,
    index: usize,
    include: &FileInclude,
    resolved: &mut ResolvedIncludes,
) -> Result<()> {
    let id = include
        .id
        .clone()
        .unwrap_or_else(|| default_include_id(&include.path, &format!("file_{index}")));
    let target = root.join(&include.path);
    let canonical = match fs::canonicalize(&target) {
        Ok(path) => path,
        Err(err) if include.optional && err.kind() == std::io::ErrorKind::NotFound => {
            resolved
                .references
                .insert(include_template_key("files", &id), String::new());
            return Ok(());
        }
        Err(err) => {
            return Err(err).with_context(|| format!("read include file {}", target.display()));
        }
    };
    if !canonical.starts_with(root) {
        bail!(
            "file include `{}` resolves outside workspace root {}",
            include.path,
            root.display()
        );
    }

    let bytes = fs::read(&canonical)
        .with_context(|| format!("read include file {}", canonical.display()))?;
    let (text, truncated) = truncate_bytes(bytes, request.options.max_include_bytes);
    resolved.references.insert(
        include_template_key("files", &id),
        include_reference(&request.spec.name, "files", &id),
    );
    resolved.blocks.push(ContextBlock {
        id: format!("command.{}.include.files.{id}", request.spec.name),
        kind: ContextBlockKind::RetrievedDocument,
        text,
        priority: 70,
        token_estimate: None,
        metadata: json!({
            "source": "command_include",
            "command": request.spec.name,
            "include_kind": "file",
            "include_id": id,
            "path": include.path,
            "truncated": truncated,
        }),
    });
    Ok(())
}

fn resolve_shell_include(
    request: &CommandExpansionRequest<'_>,
    index: usize,
    include: &ShellInclude,
    resolved: &mut ResolvedIncludes,
) -> Result<()> {
    ensure_shell_allowed(request)?;
    let runner = request.shell_runner.ok_or_else(|| {
        anyhow::anyhow!("shell include requested but no shell runner is configured")
    })?;
    let id = include
        .id
        .clone()
        .unwrap_or_else(|| default_include_id(&include.command, &format!("shell_{index}")));
    let timeout = include
        .timeout_seconds
        .unwrap_or(request.options.include_timeout_seconds);
    let output = runner.run_shell(&include.command, timeout)?;
    let (text, truncated) = truncate_bytes(output.into_bytes(), request.options.max_include_bytes);
    resolved.references.insert(
        include_template_key("shell", &id),
        include_reference(&request.spec.name, "shell", &id),
    );
    resolved.blocks.push(ContextBlock {
        id: format!("command.{}.include.shell.{id}", request.spec.name),
        kind: ContextBlockKind::Environment,
        text,
        priority: 60,
        token_estimate: None,
        metadata: json!({
            "source": "command_include",
            "command": request.spec.name,
            "include_kind": "shell",
            "include_id": id,
            "shell_command": include.command,
            "timeout_seconds": timeout,
            "truncated": truncated,
        }),
    });
    Ok(())
}

fn resolve_url_include(
    request: &CommandExpansionRequest<'_>,
    index: usize,
    include: &UrlInclude,
    resolved: &mut ResolvedIncludes,
) -> Result<()> {
    let host = ensure_url_allowed(request, &include.url)?;
    let fetcher = request
        .url_fetcher
        .ok_or_else(|| anyhow::anyhow!("URL include requested but no URL fetcher is configured"))?;
    let id = include
        .id
        .clone()
        .unwrap_or_else(|| default_include_id(&include.url, &format!("url_{index}")));
    let text = match fetcher.fetch_url(
        &include.url,
        request.options.include_timeout_seconds,
        request.options.max_include_bytes,
    ) {
        Ok(text) => text,
        Err(_) if include.optional => {
            resolved
                .references
                .insert(include_template_key("urls", &id), String::new());
            return Ok(());
        }
        Err(err) => return Err(err),
    };
    let (text, truncated) = truncate_bytes(text.into_bytes(), request.options.max_include_bytes);
    resolved.references.insert(
        include_template_key("urls", &id),
        include_reference(&request.spec.name, "urls", &id),
    );
    resolved.blocks.push(ContextBlock {
        id: format!("command.{}.include.urls.{id}", request.spec.name),
        kind: ContextBlockKind::RetrievedDocument,
        text,
        priority: 50,
        token_estimate: None,
        metadata: json!({
            "source": "command_include",
            "command": request.spec.name,
            "include_kind": "url",
            "include_id": id,
            "url": include.url,
            "host": host,
            "truncated": truncated,
        }),
    });
    Ok(())
}

fn ensure_shell_allowed(request: &CommandExpansionRequest<'_>) -> Result<()> {
    if !request.options.allow_shell_includes {
        bail!("shell includes are disabled by command configuration");
    }
    if !PolicyModeConfig::for_mode(request.options.policy_mode).allow_process {
        bail!(
            "shell includes are blocked by active policy mode {:?}",
            request.options.policy_mode
        );
    }
    Ok(())
}

fn ensure_url_allowed<'a>(request: &CommandExpansionRequest<'_>, url: &'a str) -> Result<&'a str> {
    if !request.options.allow_url_includes {
        bail!("URL includes are disabled by command configuration");
    }
    if !PolicyModeConfig::for_mode(request.options.policy_mode).allow_network {
        bail!(
            "URL includes are blocked by active policy mode {:?}",
            request.options.policy_mode
        );
    }
    let host = url_host(url).ok_or_else(|| anyhow::anyhow!("URL include `{url}` has no host"))?;
    if !request
        .options
        .allowed_url_hosts
        .iter()
        .any(|allowed| allowed == host)
    {
        bail!("URL include host `{host}` is not in the allowlist");
    }
    Ok(host)
}

fn url_host(url: &str) -> Option<&str> {
    let (_, rest) = url.split_once("://")?;
    let host = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .split('@')
        .next_back()
        .unwrap_or_default()
        .split(':')
        .next()
        .unwrap_or_default();
    (!host.is_empty()).then_some(host)
}

fn truncate_bytes(bytes: Vec<u8>, max_bytes: usize) -> (String, bool) {
    if bytes.len() <= max_bytes {
        return (String::from_utf8_lossy(&bytes).to_string(), false);
    }
    (
        String::from_utf8_lossy(&bytes[..max_bytes]).to_string(),
        true,
    )
}
