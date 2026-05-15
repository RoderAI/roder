use std::path::Path;

use anyhow::{Result, bail};
use serde::Deserialize;

use crate::spec::{CommandInclude, FileInclude, ShellInclude, UrlInclude};

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Frontmatter {
    pub name: Option<String>,
    pub description: Option<String>,
    #[serde(rename = "argument-hint")]
    pub argument_hint: Option<String>,
    #[serde(default, rename = "allowed-tools")]
    pub allowed_tools: Vec<String>,
    pub model: Option<String>,
    pub agent: Option<String>,
    #[serde(default)]
    pub include: CommandInclude,
}

pub(crate) fn parse_frontmatter(frontmatter: &str, path: &Path) -> Result<Frontmatter> {
    let parsed: Frontmatter = serde_yaml::from_str(frontmatter).map_err(|err| {
        let line = err
            .location()
            .map(|location| location.line() + 1)
            .unwrap_or(1);
        anyhow::anyhow!("{}:{line}: {}", path.display(), err)
    })?;

    validate_optional_string(path, "name", parsed.name.as_deref())?;
    validate_optional_string(path, "description", parsed.description.as_deref())?;
    validate_optional_string(path, "argument-hint", parsed.argument_hint.as_deref())?;
    validate_optional_string(path, "model", parsed.model.as_deref())?;
    validate_optional_string(path, "agent", parsed.agent.as_deref())?;
    for (index, tool) in parsed.allowed_tools.iter().enumerate() {
        if tool.trim().is_empty() {
            bail!(
                "{}: allowed-tools[{index}] must not be empty",
                path.display()
            );
        }
    }
    validate_includes(path, &parsed.include)?;
    Ok(parsed)
}

fn validate_optional_string(path: &Path, field: &str, value: Option<&str>) -> Result<()> {
    if value.is_some_and(|value| value.trim().is_empty()) {
        bail!("{}: `{field}` must not be empty", path.display());
    }
    Ok(())
}

fn validate_includes(path: &Path, include: &CommandInclude) -> Result<()> {
    for (index, file) in include.files.iter().enumerate() {
        validate_file_include(path, index, file)?;
    }
    for (index, shell) in include.shell.iter().enumerate() {
        validate_shell_include(path, index, shell)?;
    }
    for (index, url) in include.urls.iter().enumerate() {
        validate_url_include(path, index, url)?;
    }
    Ok(())
}

fn validate_file_include(path: &Path, index: usize, include: &FileInclude) -> Result<()> {
    if include.path.trim().is_empty() {
        bail!(
            "{}: include.files[{index}].path must not be empty",
            path.display()
        );
    }
    validate_optional_string(
        path,
        &format!("include.files[{index}].id"),
        include.id.as_deref(),
    )
}

fn validate_shell_include(path: &Path, index: usize, include: &ShellInclude) -> Result<()> {
    validate_optional_string(
        path,
        &format!("include.shell[{index}].id"),
        include.id.as_deref(),
    )?;
    if include.command.trim().is_empty() {
        bail!(
            "{}: include.shell[{index}].command must not be empty",
            path.display()
        );
    }
    Ok(())
}

fn validate_url_include(path: &Path, index: usize, include: &UrlInclude) -> Result<()> {
    validate_optional_string(
        path,
        &format!("include.urls[{index}].id"),
        include.id.as_deref(),
    )?;
    if include.url.trim().is_empty() {
        bail!(
            "{}: include.urls[{index}].url must not be empty",
            path.display()
        );
    }
    Ok(())
}

impl<'de> Deserialize<'de> for CommandInclude {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawInclude {
            #[serde(default)]
            files: Vec<FileInclude>,
            #[serde(default)]
            shell: Vec<ShellInclude>,
            #[serde(default)]
            urls: Vec<UrlInclude>,
        }

        let raw = RawInclude::deserialize(deserializer)?;
        Ok(CommandInclude {
            files: raw.files,
            shell: raw.shell,
            urls: raw.urls,
        })
    }
}

impl<'de> Deserialize<'de> for FileInclude {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawFileInclude {
            id: Option<String>,
            path: String,
            #[serde(default)]
            optional: bool,
        }

        let raw = RawFileInclude::deserialize(deserializer)?;
        Ok(FileInclude {
            id: raw.id,
            path: raw.path,
            optional: raw.optional,
        })
    }
}

impl<'de> Deserialize<'de> for ShellInclude {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawShellInclude {
            id: Option<String>,
            command: String,
            timeout_seconds: Option<u64>,
        }

        let raw = RawShellInclude::deserialize(deserializer)?;
        Ok(ShellInclude {
            id: raw.id,
            command: raw.command,
            timeout_seconds: raw.timeout_seconds,
        })
    }
}

impl<'de> Deserialize<'de> for UrlInclude {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawUrlInclude {
            id: Option<String>,
            url: String,
            #[serde(default)]
            optional: bool,
        }

        let raw = RawUrlInclude::deserialize(deserializer)?;
        Ok(UrlInclude {
            id: raw.id,
            url: raw.url,
            optional: raw.optional,
        })
    }
}
