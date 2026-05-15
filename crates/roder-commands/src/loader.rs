use std::{fs, path::Path};

use anyhow::{Context, Result, anyhow, bail};

use crate::{
    frontmatter::parse_frontmatter,
    spec::{CommandSource, CommandSpec},
};

pub fn load_command_file(path: impl AsRef<Path>, source: CommandSource) -> Result<CommandSpec> {
    let path = path.as_ref();
    if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
        bail!(
            "{}: command definition body must use the .md extension",
            path.display()
        );
    }
    let markdown =
        fs::read_to_string(path).with_context(|| format!("read command {}", path.display()))?;
    parse_command_markdown(&markdown, path, source)
}

pub fn parse_command_markdown(
    markdown: &str,
    path: impl AsRef<Path>,
    source: CommandSource,
) -> Result<CommandSpec> {
    let path = path.as_ref();
    let default_name = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.trim().is_empty())
        .ok_or_else(|| anyhow!("{}: command filename must have a stem", path.display()))?
        .to_string();
    let (frontmatter, body) = split_frontmatter(markdown, path)?;
    let parsed = parse_frontmatter(frontmatter, path)?;
    let name = parsed.name.unwrap_or(default_name).trim().to_string();
    if name.is_empty() {
        bail!("{}: command name must not be empty", path.display());
    }

    Ok(CommandSpec {
        name,
        description: non_empty(parsed.description),
        argument_hint: non_empty(parsed.argument_hint),
        allowed_tools: parsed.allowed_tools,
        model: non_empty(parsed.model),
        agent: non_empty(parsed.agent),
        include: parsed.include,
        body: body.trim().to_string(),
        source,
        path: Some(path.to_path_buf()),
    })
}

fn split_frontmatter<'a>(markdown: &'a str, path: &Path) -> Result<(&'a str, &'a str)> {
    let markdown = markdown.strip_prefix('\u{feff}').unwrap_or(markdown);
    let rest = markdown
        .strip_prefix("---\n")
        .or_else(|| markdown.strip_prefix("---\r\n"))
        .ok_or_else(|| {
            anyhow!(
                "{}: command definition must start with YAML frontmatter delimiter `---`",
                path.display()
            )
        })?;
    let end = rest.find("\n---\n").or_else(|| rest.find("\r\n---\r\n"));
    let Some(end) = end else {
        bail!(
            "{}: command definition is missing closing frontmatter delimiter `---`",
            path.display()
        );
    };
    let body_start = if rest[end..].starts_with("\n---\n") {
        end + "\n---\n".len()
    } else {
        end + "\r\n---\r\n".len()
    };
    Ok((&rest[..end], &rest[body_start..]))
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let value = value.trim().to_string();
        (!value.is_empty()).then_some(value)
    })
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use crate::{
        registry::{CommandsRegistry, ExtensionCommandDirectory},
        spec::CommandSource,
    };

    use super::load_command_file;

    static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn loader_parses_frontmatter_and_body() {
        let dir = tempdir("loader_parses_frontmatter_and_body");
        let path = dir.join("review.md");
        write(
            &path,
            r#"---
name: review
description: Review the current diff.
argument-hint: "[focus]"
allowed-tools: [Read, Grep, Bash]
model: claude-sonnet
agent: review
include:
  files:
    - path: "CLAUDE.md"
      optional: true
  shell:
    - id: diff
      command: "git diff HEAD"
      timeout_seconds: 5
  urls:
    - id: docs
      url: "https://example.com/docs"
      optional: false
---

Review {{arguments}}.
"#,
        );

        let spec = load_command_file(&path, CommandSource::Workspace).unwrap();
        assert_eq!(spec.name, "review");
        assert_eq!(
            spec.description.as_deref(),
            Some("Review the current diff.")
        );
        assert_eq!(spec.argument_hint.as_deref(), Some("[focus]"));
        assert_eq!(spec.allowed_tools, ["Read", "Grep", "Bash"]);
        assert_eq!(spec.model.as_deref(), Some("claude-sonnet"));
        assert_eq!(spec.agent.as_deref(), Some("review"));
        assert_eq!(spec.include.files[0].path, "CLAUDE.md");
        assert!(spec.include.files[0].optional);
        assert_eq!(spec.include.shell[0].id.as_deref(), Some("diff"));
        assert_eq!(spec.include.shell[0].timeout_seconds, Some(5));
        assert_eq!(spec.include.urls[0].id.as_deref(), Some("docs"));
        assert_eq!(spec.body, "Review {{arguments}}.");
    }

    #[test]
    fn loader_defaults_name_to_file_stem() {
        let dir = tempdir("loader_defaults_name_to_file_stem");
        let path = dir.join("explain.more.md");
        write(&path, "---\ndescription: Explain\n---\n\nExplain it.");

        let spec = load_command_file(&path, CommandSource::User).unwrap();
        assert_eq!(spec.name, "explain.more");
    }

    #[test]
    fn loader_malformed_frontmatter_reports_path_and_line() {
        let dir = tempdir("loader_malformed_frontmatter_reports_path_and_line");
        let path = dir.join("broken.md");
        write(
            &path,
            "---\ndescription: ok\nallowed-tools: Read\n---\nBody",
        );

        let err = load_command_file(&path, CommandSource::User)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains(&format!("{}:3:", path.display())),
            "error should contain path and line, got {err}"
        );
        assert!(err.contains("invalid type"), "unexpected error: {err}");
    }

    #[test]
    fn loader_rejects_unknown_fields() {
        let dir = tempdir("loader_rejects_unknown_fields");
        let path = dir.join("broken.md");
        write(&path, "---\ndescripton: typo\n---\nBody");

        let err = load_command_file(&path, CommandSource::User)
            .unwrap_err()
            .to_string();
        assert!(err.contains(&format!("{}:2:", path.display())), "{err}");
        assert!(err.contains("unknown field"), "{err}");
    }

    #[test]
    fn loader_supports_bom_and_crlf() {
        let dir = tempdir("loader_supports_bom_and_crlf");
        let path = dir.join("review.md");
        write(
            &path,
            "\u{feff}---\r\nname: review\r\ndescription: CRLF\r\n---\r\nBody\r\n",
        );

        let spec = load_command_file(&path, CommandSource::User).unwrap();
        assert_eq!(spec.name, "review");
        assert_eq!(spec.body, "Body");
    }

    #[test]
    fn loader_rejects_non_markdown_files() {
        let dir = tempdir("loader_rejects_non_markdown_files");
        let path = dir.join("review");
        write(&path, "---\nname: review\n---\nBody");

        let err = load_command_file(&path, CommandSource::User)
            .unwrap_err()
            .to_string();
        assert!(err.contains("must use the .md extension"), "{err}");
    }

    #[test]
    fn loader_registry_workspace_overrides_user() {
        let dir = tempdir("loader_registry_workspace_overrides_user");
        let user = dir.join("user");
        let workspace = dir.join("workspace");
        write(
            &user.join("review.md"),
            "---\ndescription: user\n---\n\nUser body",
        );
        write(
            &workspace.join("review.md"),
            "---\ndescription: workspace\n---\n\nWorkspace body",
        );

        let registry = CommandsRegistry::load(Some(&user), Some(&workspace), []).unwrap();
        let spec = registry.get("review").unwrap();
        assert_eq!(spec.description.as_deref(), Some("workspace"));
        assert_eq!(spec.source, CommandSource::Workspace);
    }

    #[test]
    fn loader_rejects_duplicate_workspace_command_names() {
        let dir = tempdir("loader_rejects_duplicate_workspace_command_names");
        let workspace = dir.join("workspace");
        write(&workspace.join("one.md"), "---\nname: review\n---\n\nOne");
        write(&workspace.join("two.md"), "---\nname: review\n---\n\nTwo");

        let err = CommandsRegistry::load(None::<&PathBuf>, Some(&workspace), [])
            .unwrap_err()
            .to_string();
        assert!(err.contains("duplicate command `review`"), "{err}");
        assert!(err.contains("one.md") && err.contains("two.md"), "{err}");
    }

    #[test]
    fn loader_registry_iterates_in_command_name_order() {
        let dir = tempdir("loader_registry_iterates_in_command_name_order");
        let workspace = dir.join("workspace");
        write(&workspace.join("zeta.md"), "---\nname: zeta\n---\n\nZ");
        write(&workspace.join("alpha.md"), "---\nname: alpha\n---\n\nA");

        let registry = CommandsRegistry::load_with_options(
            None::<&PathBuf>,
            Some(&workspace),
            [],
            crate::registry::CommandsRegistryOptions {
                include_builtins: false,
                ..crate::registry::CommandsRegistryOptions::default()
            },
        )
        .unwrap();
        let names = registry
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, ["alpha", "zeta"]);
    }

    #[test]
    fn loader_enforces_extension_namespace_and_collisions() {
        let dir = tempdir("loader_enforces_extension_namespace_and_collisions");
        let bad = dir.join("bad");
        write(&bad.join("review.md"), "---\nname: review\n---\n\nBad");

        let err = CommandsRegistry::load(
            None::<&PathBuf>,
            None::<&PathBuf>,
            [ExtensionCommandDirectory {
                extension_id: "lint".to_string(),
                root: bad.clone(),
            }],
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("extension command `review` must use namespace `ext.lint.`"));

        let first = dir.join("first");
        let second = dir.join("second");
        write(
            &first.join("review.md"),
            "---\nname: ext.lint.review\n---\n\nFirst",
        );
        write(
            &second.join("review.md"),
            "---\nname: ext.lint.review\n---\n\nSecond",
        );
        let err = CommandsRegistry::load(
            None::<&PathBuf>,
            None::<&PathBuf>,
            [
                ExtensionCommandDirectory {
                    extension_id: "lint".to_string(),
                    root: first,
                },
                ExtensionCommandDirectory {
                    extension_id: "lint".to_string(),
                    root: second,
                },
            ],
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("duplicate command `ext.lint.review`"), "{err}");
    }

    fn tempdir(name: &str) -> PathBuf {
        let unique = TEMP_COUNTER.fetch_add(1, Ordering::SeqCst);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "roder-commands-{name}-{}-{nanos}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn write(path: &PathBuf, content: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
}
