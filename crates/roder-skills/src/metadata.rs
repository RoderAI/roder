use std::path::Path;

use anyhow::Context;
use roder_api::skills::{
    Skill, SkillActivationState, SkillAgentMetadata, SkillDescriptor, SkillExposure, SkillSource,
};
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
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct SkillFrontmatterMetadata {
    short_description: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenAiAgentYaml {
    interface: Option<String>,
    #[serde(default)]
    dependencies: Vec<String>,
    #[serde(default, alias = "policy")]
    policies: Vec<String>,
}

pub fn parse_skill_markdown(text: &str) -> anyhow::Result<ParsedSkillMarkdown> {
    let Some(rest) = text.strip_prefix("---\n") else {
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
        _ => {
            diagnostics.push("missing required frontmatter field: name".to_string());
            "unnamed".to_string()
        }
    };
    let description = match raw.description {
        Some(description) if !description.trim().is_empty() => description.trim().to_string(),
        _ => {
            diagnostics.push("missing required frontmatter field: description".to_string());
            String::new()
        }
    };
    Ok(ParsedSkillMarkdown {
        name,
        description,
        short_description: raw.metadata.short_description,
        exposure: raw.exposure,
        experimental: raw.experimental,
        body: body.trim_start_matches('\n').to_string(),
        diagnostics,
    })
}

pub fn parse_openai_agent_yaml(text: &str) -> anyhow::Result<SkillAgentMetadata> {
    let parsed: OpenAiAgentYaml = serde_yaml::from_str(text).context("parse agents/openai.yaml")?;
    let raw =
        serde_yaml::from_str::<serde_json::Value>(text).unwrap_or_else(|_| serde_json::Value::Null);
    Ok(SkillAgentMetadata {
        interface: parsed.interface,
        dependencies: parsed.dependencies,
        policies: parsed.policies,
        raw,
    })
}

pub fn load_skill_from_paths(
    skill_path: &Path,
    source: SkillSource,
    canonical_path: String,
    default_exposure: SkillExposure,
) -> anyhow::Result<Skill> {
    let text = std::fs::read_to_string(skill_path)
        .with_context(|| format!("read skill file {}", skill_path.display()))?;
    let parsed = parse_skill_markdown(&text)?;
    let agent_path = skill_path
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join("agents")
        .join("openai.yaml");
    let agent_metadata = if agent_path.exists() {
        Some(parse_openai_agent_yaml(
            &std::fs::read_to_string(&agent_path)
                .with_context(|| format!("read agent metadata {}", agent_path.display()))?,
        )?)
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
}
