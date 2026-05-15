use anyhow::{Context, bail};
use roder_api::subagents::{SubagentDefinition, SubagentPermissionMode};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentDefinitionSource {
    pub definition: SubagentDefinition,
    pub source_path: Option<std::path::PathBuf>,
}

#[derive(Debug, Default)]
struct Frontmatter {
    name: Option<String>,
    description: Option<String>,
    model: Option<String>,
    tools: Vec<String>,
    permission_mode: Option<SubagentPermissionMode>,
    max_turns: Option<u32>,
    max_result_chars: Option<usize>,
}

pub fn parse_agent_definition(markdown: &str) -> anyhow::Result<SubagentDefinition> {
    let (frontmatter, body) = split_frontmatter(markdown)?;
    let parsed = parse_frontmatter(frontmatter)?;
    let name = parsed
        .name
        .filter(|name| !name.trim().is_empty())
        .context("agent definition is missing required frontmatter field `name`")?;
    let description = parsed
        .description
        .filter(|description| !description.trim().is_empty())
        .context("agent definition is missing required frontmatter field `description`")?;
    if parsed.tools.is_empty() {
        bail!("agent definition {name:?} must declare at least one tool");
    }
    let system_prompt = (!body.trim().is_empty()).then(|| body.trim().to_string());

    Ok(SubagentDefinition {
        agent_type: name,
        description,
        tools: parsed.tools,
        model: parsed.model.filter(|model| !model.trim().is_empty()),
        system_prompt,
        permission_mode: parsed.permission_mode.unwrap_or_default(),
        max_turns: parsed.max_turns,
        max_result_chars: parsed.max_result_chars,
    })
}

fn split_frontmatter(markdown: &str) -> anyhow::Result<(&str, &str)> {
    let markdown = markdown.strip_prefix('\u{feff}').unwrap_or(markdown);
    let rest = markdown
        .strip_prefix("---\n")
        .or_else(|| markdown.strip_prefix("---\r\n"))
        .context("agent definition must start with markdown frontmatter delimiter `---`")?;
    let end = rest
        .find("\n---\n")
        .or_else(|| rest.find("\r\n---\r\n"))
        .context("agent definition is missing closing frontmatter delimiter `---`")?;
    let frontmatter = &rest[..end];
    let body_start = if rest[end..].starts_with("\n---\n") {
        end + "\n---\n".len()
    } else {
        end + "\r\n---\r\n".len()
    };
    Ok((frontmatter, &rest[body_start..]))
}

fn parse_frontmatter(frontmatter: &str) -> anyhow::Result<Frontmatter> {
    let mut parsed = Frontmatter::default();
    let mut block_list_key: Option<&str> = None;
    for raw_line in frontmatter.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(key) = block_list_key
            && let Some(item) = line.strip_prefix("- ")
            && key == "tools"
        {
            parsed.tools.push(unquote(item));
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            bail!("invalid frontmatter line {line:?}");
        };
        let key = key.trim();
        let value = value.trim();
        block_list_key = None;
        match key {
            "name" | "agent_type" => parsed.name = Some(unquote(value)),
            "description" => parsed.description = Some(unquote(value)),
            "model" => parsed.model = Some(unquote(value)),
            "tools" => {
                if value.is_empty() {
                    block_list_key = Some("tools");
                } else {
                    parsed.tools = parse_tools(value)?;
                }
            }
            "permission_mode" => parsed.permission_mode = Some(parse_permission_mode(value)?),
            "max_turns" => parsed.max_turns = Some(unquote(value).parse()?),
            "max_result_chars" => parsed.max_result_chars = Some(unquote(value).parse()?),
            _ => {}
        }
    }
    Ok(parsed)
}

fn parse_tools(value: &str) -> anyhow::Result<Vec<String>> {
    let value = value.trim();
    if value.starts_with('[') && value.ends_with(']') {
        let inner = &value[1..value.len() - 1];
        return Ok(inner
            .split(',')
            .map(unquote)
            .filter(|tool| !tool.is_empty())
            .collect());
    }
    if value.is_empty() {
        return Ok(Vec::new());
    }
    Ok(vec![unquote(value)])
}

fn parse_permission_mode(value: &str) -> anyhow::Result<SubagentPermissionMode> {
    match unquote(value).as_str() {
        "read_only" => Ok(SubagentPermissionMode::ReadOnly),
        "default" => Ok(SubagentPermissionMode::Default),
        "auto_edit" => Ok(SubagentPermissionMode::AutoEdit),
        other => bail!("unknown permission_mode {other:?}"),
    }
}

fn unquote(value: &str) -> String {
    value
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .to_string()
}
