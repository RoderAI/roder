use std::path::Path;

use anyhow::Context;
use roder_api::skills::{
    Skill, SkillActivationState, SkillAgentMetadata, SkillDescriptor, SkillExposure, SkillSource,
};

fn fallback_name_for_path(skill_path: &Path) -> Option<String> {
    skill_path
        .parent()
        .and_then(|parent| parent.file_name())
        .map(|name| name.to_string_lossy().to_string())
}

fn plain_markdown_skill(text: &str, fallback_name: Option<&str>) -> ParsedSkillMarkdown {
    let name = fallback_name.unwrap_or("unnamed").to_string();
    ParsedSkillMarkdown {
        name,
        description: summarize_plain_markdown(text, fallback_name),
        short_description: None,
        exposure: None,
        experimental: false,
        body: text.to_string(),
        diagnostics: vec!["loaded plain markdown without SKILL.md frontmatter".to_string()],
    }
}

fn fallback_description(fallback_name: Option<&str>) -> String {
    fallback_name
        .map(|name| format!("Local skill {name}"))
        .unwrap_or_default()
}

fn summarize_plain_markdown(text: &str, fallback_name: Option<&str>) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.trim_start_matches('#').trim().to_string())
        .filter(|line| !line.is_empty())
        .unwrap_or_else(|| fallback_description(fallback_name))
}

fn extract_interface(raw: &serde_json::Value) -> Option<String> {
    let value = raw.get("interface")?;
    if let Some(text) = value.as_str() {
        return Some(text.to_string());
    }
    value
        .get("display_name")
        .or_else(|| value.get("displayName"))
        .or_else(|| value.get("short_description"))
        .or_else(|| value.get("shortDescription"))
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string)
}

fn extract_dependencies(raw: &serde_json::Value) -> Vec<String> {
    match raw.get("dependencies") {
        Some(serde_json::Value::Array(values)) => collect_string_array(values),
        Some(serde_json::Value::Object(map)) => map
            .get("tools")
            .and_then(serde_json::Value::as_array)
            .map(|values| collect_string_array(values))
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn extract_policies(raw: &serde_json::Value) -> Vec<String> {
    let Some(value) = raw.get("policy").or_else(|| raw.get("policies")) else {
        return Vec::new();
    };
    match value {
        serde_json::Value::Array(values) => collect_string_array(values),
        serde_json::Value::Object(map) => map
            .iter()
            .filter_map(|(key, value)| match value {
                serde_json::Value::Bool(flag) => Some(format!("{key}={flag}")),
                serde_json::Value::String(text) => Some(format!("{key}={text}")),
                serde_json::Value::Array(values) => {
                    Some(format!("{key}={}", collect_string_array(values).join(",")))
                }
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn collect_string_array(values: &[serde_json::Value]) -> Vec<String> {
    values
        .iter()
        .filter_map(serde_json::Value::as_str)
        .map(ToString::to_string)
        .collect()
}
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSkillMarkdown {
    pub name: String,
    pub description: String,
    pub short_description: Option<String>,
    pub exposure: Option<SkillExposure>,
    pub experimental: bool,
    pub body: String,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct SkillFrontmatter {
    name: Option<String>,
    description: Option<String>,
    #[serde(default)]
    metadata: SkillFrontmatterMetadata,
    #[serde(default)]
    exposure: Option<SkillExposure>,
    #[serde(default)]
    experimental: bool,
    #[serde(default, alias = "alwaysApply", alias = "always_apply")]
    always_apply: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct SkillFrontmatterMetadata {
    #[serde(alias = "shortDescription", alias = "short_description")]
    short_description: Option<String>,
}

pub fn parse_skill_markdown(text: &str) -> anyhow::Result<ParsedSkillMarkdown> {
    parse_skill_markdown_with_fallback(text, None, false)
}

fn parse_skill_markdown_with_fallback(
    text: &str,
    fallback_name: Option<&str>,
    allow_plain_markdown: bool,
) -> anyhow::Result<ParsedSkillMarkdown> {
    let Some(rest) = text.strip_prefix("---\n") else {
        if allow_plain_markdown {
            return Ok(plain_markdown_skill(text, fallback_name));
        }
        anyhow::bail!("SKILL.md is missing YAML frontmatter");
    };
    let Some((frontmatter, body)) = rest.split_once("\n---") else {
        anyhow::bail!("SKILL.md frontmatter is not closed");
    };
    let raw: SkillFrontmatter =
        serde_yaml::from_str(frontmatter).context("parse SKILL.md frontmatter")?;
    let mut diagnostics = Vec::new();
    let name = match raw.name {
        Some(name) if !name.trim().is_empty() => name.trim().to_string(),
        _ if fallback_name.is_some() => fallback_name.unwrap_or_default().to_string(),
        _ => {
            diagnostics.push("missing required frontmatter field: name".to_string());
            "unnamed".to_string()
        }
    };
    let description = match raw.description {
        Some(description) if !description.trim().is_empty() => description.trim().to_string(),
        _ if raw.metadata.short_description.is_some() => raw
            .metadata
            .short_description
            .as_deref()
            .unwrap_or_default()
            .trim()
            .to_string(),
        _ => {
            diagnostics.push("missing required frontmatter field: description".to_string());
            fallback_description(fallback_name)
        }
    };
    let exposure = raw.exposure.or(match raw.always_apply {
        Some(true) => Some(SkillExposure::Global),
        Some(false) => Some(SkillExposure::DirectOnly),
        None => None,
    });
    Ok(ParsedSkillMarkdown {
        name,
        description,
        short_description: raw.metadata.short_description,
        exposure,
        experimental: raw.experimental,
        body: body.trim_start_matches('\n').to_string(),
        diagnostics,
    })
}

pub fn parse_openai_agent_yaml(text: &str) -> anyhow::Result<SkillAgentMetadata> {
    let raw =
        serde_yaml::from_str::<serde_json::Value>(text).context("parse agents/openai.yaml")?;
    Ok(SkillAgentMetadata {
        interface: extract_interface(&raw),
        dependencies: extract_dependencies(&raw),
        policies: extract_policies(&raw),
        raw,
    })
}

pub fn load_skill_from_paths(
    skill_path: &Path,
    source: SkillSource,
    canonical_path: String,
    default_exposure: SkillExposure,
) -> anyhow::Result<Skill> {
    load_skill_from_paths_with_options(
        skill_path,
        source,
        canonical_path,
        default_exposure,
        fallback_name_for_path(skill_path).as_deref(),
        false,
    )
}

pub fn load_compatible_skill_from_path(
    skill_path: &Path,
    source: SkillSource,
    canonical_path: String,
    default_exposure: SkillExposure,
    fallback_name: &str,
    allow_plain_markdown: bool,
) -> anyhow::Result<Skill> {
    load_skill_from_paths_with_options(
        skill_path,
        source,
        canonical_path,
        default_exposure,
        Some(fallback_name),
        allow_plain_markdown,
    )
}

fn load_skill_from_paths_with_options(
    skill_path: &Path,
    source: SkillSource,
    canonical_path: String,
    default_exposure: SkillExposure,
    fallback_name: Option<&str>,
    allow_plain_markdown: bool,
) -> anyhow::Result<Skill> {
    let text = std::fs::read_to_string(skill_path)
        .with_context(|| format!("read skill file {}", skill_path.display()))?;
    let mut parsed =
        parse_skill_markdown_with_fallback(&text, fallback_name, allow_plain_markdown)?;
    let agent_path = skill_path
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join("agents")
        .join("openai.yaml");
    let agent_metadata = if agent_path.exists() {
        match std::fs::read_to_string(&agent_path)
            .with_context(|| format!("read agent metadata {}", agent_path.display()))
            .and_then(|text| parse_openai_agent_yaml(&text))
        {
            Ok(metadata) => Some(metadata),
            Err(err) => {
                parsed
                    .diagnostics
                    .push(format!("ignored agents/openai.yaml: {err}"));
                None
            }
        }
    } else {
        None
    };
    let id = format!("{}:{canonical_path}", source_id(&source));
    Ok(skill_from_parsed(
        parsed,
        source,
        canonical_path,
        default_exposure,
        agent_metadata,
        id,
    ))
}

pub fn skill_from_markdown(
    text: &str,
    source: SkillSource,
    canonical_path: String,
    default_exposure: SkillExposure,
    agent_metadata: Option<SkillAgentMetadata>,
) -> anyhow::Result<Skill> {
    let parsed = parse_skill_markdown(text)?;
    let id = format!("{}:{canonical_path}", source_id(&source));
    Ok(skill_from_parsed(
        parsed,
        source,
        canonical_path,
        default_exposure,
        agent_metadata,
        id,
    ))
}

fn skill_from_parsed(
    parsed: ParsedSkillMarkdown,
    source: SkillSource,
    canonical_path: String,
    default_exposure: SkillExposure,
    agent_metadata: Option<SkillAgentMetadata>,
    id: String,
) -> Skill {
    let activation = if parsed.experimental {
        SkillActivationState::Experimental
    } else {
        SkillActivationState::Enabled
    };
    Skill {
        descriptor: SkillDescriptor {
            id,
            name: parsed.name,
            canonical_path,
            source,
            exposure: parsed.exposure.unwrap_or(default_exposure),
            activation,
            description: parsed.description,
            short_description: parsed.short_description,
            experimental: parsed.experimental,
            diagnostics: parsed.diagnostics,
            agent_metadata,
        },
        body: parsed.body,
    }
}

fn source_id(source: &SkillSource) -> &'static str {
    match source {
        SkillSource::Workspace => "workspace",
        SkillSource::User => "user",
        SkillSource::Plugin { .. } => "plugin",
        SkillSource::Imported { .. } => "imported",
        SkillSource::BuiltIn => "builtin",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_parses_skill_frontmatter_and_body() {
        let parsed = parse_skill_markdown(
            "---\nname: commit\ndescription: Commit safely\nmetadata:\n  short-description: Git commit guide\nexposure: direct_only\n---\nUse git status.\n",
        )
        .unwrap();

        assert_eq!(parsed.name, "commit");
        assert_eq!(
            parsed.short_description.as_deref(),
            Some("Git commit guide")
        );
        assert_eq!(parsed.exposure, Some(SkillExposure::DirectOnly));
        assert!(parsed.body.contains("git status"));
    }

    #[test]
    fn metadata_parses_optional_openai_agent_yaml() {
        let agent = parse_openai_agent_yaml(
            "interface: openai\ndependencies:\n  - git\npolicies:\n  - do-not-stage-unrequested-files\n",
        )
        .unwrap();

        assert_eq!(agent.interface.as_deref(), Some("openai"));
        assert_eq!(agent.dependencies, vec!["git"]);
        assert_eq!(agent.policies[0], "do-not-stage-unrequested-files");
    }

    #[test]
    fn metadata_parses_rich_agent_yaml_without_rejecting_the_skill() {
        let agent = parse_openai_agent_yaml(
            "interface:\n  display_name: Review\n  short_description: Review code\ndependencies:\n  tools:\n    - git\npolicy:\n  allow_implicit_invocation: false\n",
        )
        .unwrap();

        assert_eq!(agent.interface.as_deref(), Some("Review"));
        assert_eq!(agent.dependencies, vec!["git"]);
        assert!(
            agent
                .policies
                .contains(&"allow_implicit_invocation=false".to_string())
        );
    }
}
