use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};

use crate::{
    built_in_commands,
    loader::load_command_file,
    spec::{CommandSource, CommandSpec},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandDirectory {
    pub root: PathBuf,
    pub source: CommandSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionCommandDirectory {
    pub extension_id: String,
    pub root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandsRegistryOptions {
    pub include_builtins: bool,
    pub allow_builtin_override: bool,
}

impl Default for CommandsRegistryOptions {
    fn default() -> Self {
        Self {
            include_builtins: true,
            allow_builtin_override: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOverrideAudit {
    pub name: String,
    pub overridden_source: CommandSource,
    pub replacement_source: CommandSource,
    pub replacement_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandsRegistry {
    commands: BTreeMap<String, CommandSpec>,
    override_audits: Vec<CommandOverrideAudit>,
    options: CommandsRegistryOptions,
}

impl CommandsRegistry {
    pub fn load(
        user_dir: Option<impl AsRef<Path>>,
        workspace_dir: Option<impl AsRef<Path>>,
        extension_dirs: impl IntoIterator<Item = ExtensionCommandDirectory>,
    ) -> Result<Self> {
        Self::load_with_options(
            user_dir,
            workspace_dir,
            extension_dirs,
            CommandsRegistryOptions::default(),
        )
    }

    pub fn load_with_options(
        user_dir: Option<impl AsRef<Path>>,
        workspace_dir: Option<impl AsRef<Path>>,
        extension_dirs: impl IntoIterator<Item = ExtensionCommandDirectory>,
        options: CommandsRegistryOptions,
    ) -> Result<Self> {
        let mut directories = Vec::new();
        if let Some(user_dir) = user_dir {
            directories.push(CommandDirectory {
                root: user_dir.as_ref().to_path_buf(),
                source: CommandSource::User,
            });
        }
        if let Some(workspace_dir) = workspace_dir {
            directories.push(CommandDirectory {
                root: workspace_dir.as_ref().to_path_buf(),
                source: CommandSource::Workspace,
            });
        }
        for extension in extension_dirs {
            directories.push(CommandDirectory {
                root: extension.root,
                source: CommandSource::Extension {
                    extension_id: extension.extension_id,
                },
            });
        }
        Self::from_directories_with_options(directories, options)
    }

    pub fn from_directories(
        directories: impl IntoIterator<Item = CommandDirectory>,
    ) -> Result<Self> {
        Self::from_directories_with_options(directories, CommandsRegistryOptions::default())
    }

    pub fn from_directories_with_options(
        directories: impl IntoIterator<Item = CommandDirectory>,
        options: CommandsRegistryOptions,
    ) -> Result<Self> {
        let mut registry = CommandsRegistry {
            commands: BTreeMap::new(),
            override_audits: Vec::new(),
            options,
        };
        if registry.options.include_builtins {
            for command in built_in_commands() {
                registry.insert(command)?;
            }
        }
        for directory in directories {
            let commands = scan_directory(&directory.root, directory.source)?;
            for command in commands {
                registry.insert(command)?;
            }
        }
        Ok(registry)
    }

    pub fn get(&self, name: &str) -> Option<&CommandSpec> {
        self.commands.get(name)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &CommandSpec)> {
        self.commands.iter()
    }

    pub fn into_commands(self) -> BTreeMap<String, CommandSpec> {
        self.commands
    }

    pub fn override_audits(&self) -> &[CommandOverrideAudit] {
        &self.override_audits
    }

    fn insert(&mut self, spec: CommandSpec) -> Result<()> {
        enforce_extension_namespace(&spec)?;
        let name = spec.name.clone();
        let Some(existing) = self.commands.get(&name) else {
            self.commands.insert(name, spec);
            return Ok(());
        };
        if existing.source == CommandSource::User && spec.source == CommandSource::Workspace {
            self.commands.insert(name, spec);
            return Ok(());
        }
        if existing.source == CommandSource::BuiltIn && self.options.allow_builtin_override {
            self.override_audits.push(CommandOverrideAudit {
                name: name.clone(),
                overridden_source: existing.source.clone(),
                replacement_source: spec.source.clone(),
                replacement_path: spec.path.clone(),
            });
            self.commands.insert(name, spec);
            return Ok(());
        }

        bail!(
            "duplicate command `{}` from {} conflicts with {}",
            name,
            describe(&spec),
            describe(existing)
        );
    }
}

fn scan_directory(root: &Path, source: CommandSource) -> Result<Vec<CommandSpec>> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    if !root.is_dir() {
        bail!("command source {} is not a directory", root.display());
    }

    let mut files = Vec::new();
    collect_markdown_files(root, &mut files)?;
    files.sort();

    let mut commands = Vec::with_capacity(files.len());
    for path in files {
        commands.push(load_command_file(&path, source.clone())?);
    }
    Ok(commands)
}

fn collect_markdown_files(root: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    let mut entries = fs::read_dir(root)
        .with_context(|| format!("read command directory {}", root.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .with_context(|| format!("read command directory {}", root.display()))?;
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let path = entry.path();
        let file_type = entry
            .file_type()
            .with_context(|| format!("read command file type {}", path.display()))?;
        if file_type.is_dir() {
            collect_markdown_files(&path, files)?;
        } else if file_type.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some("md")
        {
            files.push(path);
        }
    }
    Ok(())
}

fn enforce_extension_namespace(spec: &CommandSpec) -> Result<()> {
    if let CommandSource::Extension { extension_id } = &spec.source {
        let prefix = format!("ext.{extension_id}.");
        if !spec.name.starts_with(&prefix) {
            bail!(
                "extension command `{}` must use namespace `{}`",
                spec.name,
                prefix
            );
        }
    }
    Ok(())
}

fn describe(spec: &CommandSpec) -> String {
    let path = spec
        .path
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "<in-memory>".to_string());
    format!("{} command at {path}", spec.display_source())
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use crate::spec::CommandSource;

    use super::{CommandsRegistry, CommandsRegistryOptions};

    static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn builtin_registry_available_without_command_directories() {
        let registry = CommandsRegistry::load(None::<&PathBuf>, None::<&PathBuf>, []).unwrap();

        for name in [
            "init",
            "clear",
            "compact",
            "help",
            "goal",
            "retry",
            "model",
            "agents",
            "tasks",
            "memory",
            "commit",
            "marketplace",
            "plugin",
        ] {
            let spec = registry
                .get(name)
                .unwrap_or_else(|| panic!("missing {name}"));
            assert_eq!(spec.source, CommandSource::BuiltIn);
            assert!(!spec.body.is_empty());
        }
        assert_eq!(
            registry
                .get("marketplace")
                .and_then(|spec| spec.argument_hint.as_deref()),
            Some("list|install-default|add|remove|refresh|search|show [args]")
        );
        assert_eq!(
            registry
                .get("plugin")
                .and_then(|spec| spec.argument_hint.as_deref()),
            Some("preview|install|install-all|list|disable|uninstall [args]")
        );
    }

    #[test]
    fn builtin_override_requires_explicit_option_and_records_audit() {
        let dir = tempdir("builtin_override_requires_explicit_option_and_records_audit");
        write(&dir.join("help.md"), "---\nname: help\n---\n\nProject help");

        let err = CommandsRegistry::load(None::<&PathBuf>, Some(&dir), [])
            .unwrap_err()
            .to_string();
        assert!(err.contains("duplicate command `help`"), "{err}");
        assert!(err.contains("built-in command"), "{err}");

        let registry = CommandsRegistry::load_with_options(
            None::<&PathBuf>,
            Some(&dir),
            [],
            CommandsRegistryOptions {
                allow_builtin_override: true,
                ..CommandsRegistryOptions::default()
            },
        )
        .unwrap();
        let spec = registry.get("help").unwrap();
        assert_eq!(spec.source, CommandSource::Workspace);
        assert_eq!(spec.body, "Project help");
        assert_eq!(registry.override_audits().len(), 1);
        assert_eq!(registry.override_audits()[0].name, "help");
        assert_eq!(
            registry.override_audits()[0].overridden_source,
            CommandSource::BuiltIn
        );
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
