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
    workflows::{WorkflowCommandDirectory, scan_workflow_directory},
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

#[derive(Debug, Clone, Default, PartialEq)]
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
        Self::from_directories_with_workflows(directories, std::iter::empty(), options)
    }

    pub fn from_directories_with_workflows(
        directories: impl IntoIterator<Item = CommandDirectory>,
        workflow_directories: impl IntoIterator<Item = WorkflowCommandDirectory>,
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
        for directory in workflow_directories {
            let commands = scan_workflow_directory(&directory.root, directory.source)?;
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
        // Package commands load flat but never beat built-in, user, or
        // workspace commands: the higher-priority side wins in both load
        // orders, with an audit entry recording the shadowing. Two packages
        // shipping the same command name is a hard conflict.
        let existing_shadows_package = matches!(spec.source, CommandSource::Package { .. })
            && matches!(
                existing.source,
                CommandSource::BuiltIn | CommandSource::User | CommandSource::Workspace
            );
        if existing_shadows_package {
            self.override_audits.push(CommandOverrideAudit {
                name,
                overridden_source: spec.source.clone(),
                replacement_source: existing.source.clone(),
                replacement_path: existing.path.clone(),
            });
            return Ok(());
        }
        let package_yields_to_new = matches!(existing.source, CommandSource::Package { .. })
            && matches!(
                spec.source,
                CommandSource::User | CommandSource::Workspace
            );
        if package_yields_to_new {
            self.override_audits.push(CommandOverrideAudit {
                name: name.clone(),
                overridden_source: existing.source.clone(),
                replacement_source: spec.source.clone(),
                replacement_path: spec.path.clone(),
            });
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
            "ps",
            "memory",
            "snapshot",
            "marketplace",
            "plugin",
            "packages",
            "remote",
            "voice",
            "roadmap",
            "deep-research",
        ] {
            let spec = registry
                .get(name)
                .unwrap_or_else(|| panic!("missing {name}"));
            assert_eq!(spec.source, CommandSource::BuiltIn);
            assert!(!spec.body.is_empty());
        }
        assert_eq!(
            registry
                .get("ps")
                .and_then(|spec| spec.argument_hint.as_deref()),
            Some("all|stop <id>|stop-all --confirm|<id>")
        );
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
        assert_eq!(
            registry
                .get("packages")
                .and_then(|spec| spec.argument_hint.as_deref()),
            Some("list|install|remove|update|enable|disable|approve|sync [args]")
        );
        assert_eq!(
            registry
                .get("voice")
                .and_then(|spec| spec.argument_hint.as_deref()),
            Some("[hold|tap|off|status]")
        );
        assert_eq!(
            registry
                .get("roadmap")
                .and_then(|spec| spec.argument_hint.as_deref()),
            Some("[plan]")
        );
        assert_eq!(
            registry
                .get("deep-research")
                .and_then(|spec| spec.argument_hint.as_deref()),
            Some("<question>")
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

    #[test]
    fn package_commands_load_flat_and_yield_to_user_and_workspace() {
        let dir = tempdir("package_commands_load_flat_and_yield_to_user_and_workspace");
        let package = dir.join("package");
        let user = dir.join("user");
        write(&package.join("greet.md"), "---\nname: greet\n---\n\nPkg");
        write(&package.join("only.md"), "---\nname: only\n---\n\nOnly");
        write(&user.join("greet.md"), "---\nname: greet\n---\n\nUser");

        let package_dir = super::CommandDirectory {
            root: package.clone(),
            source: CommandSource::Package {
                package_id: "demo-pkg".to_string(),
            },
        };

        // Package loads after user: user keeps the name.
        let registry = CommandsRegistry::from_directories([
            super::CommandDirectory {
                root: user.clone(),
                source: CommandSource::User,
            },
            package_dir.clone(),
        ])
        .unwrap();
        assert_eq!(registry.get("greet").unwrap().body, "User");
        assert_eq!(
            registry.get("only").unwrap().source,
            CommandSource::Package {
                package_id: "demo-pkg".to_string()
            }
        );
        assert_eq!(registry.override_audits().len(), 1);

        // Package loads before user: user replaces the package command.
        let registry = CommandsRegistry::from_directories([
            package_dir,
            super::CommandDirectory {
                root: user,
                source: CommandSource::User,
            },
        ])
        .unwrap();
        assert_eq!(registry.get("greet").unwrap().body, "User");
        assert_eq!(registry.override_audits().len(), 1);
    }

    #[test]
    fn package_command_never_overrides_builtin_and_conflicts_with_other_package() {
        let dir = tempdir("package_command_never_overrides_builtin");
        let package = dir.join("package");
        write(&package.join("help.md"), "---\nname: help\n---\n\nPkg help");

        let registry = CommandsRegistry::from_directories([super::CommandDirectory {
            root: package,
            source: CommandSource::Package {
                package_id: "demo-pkg".to_string(),
            },
        }])
        .unwrap();
        // Built-in stays; the package command is shadowed with an audit.
        assert_eq!(registry.get("help").unwrap().source, CommandSource::BuiltIn);
        assert_eq!(registry.override_audits().len(), 1);

        let first = dir.join("first");
        let second = dir.join("second");
        write(&first.join("greet.md"), "---\nname: greet\n---\n\nOne");
        write(&second.join("greet.md"), "---\nname: greet\n---\n\nTwo");
        let err = CommandsRegistry::from_directories([
            super::CommandDirectory {
                root: first,
                source: CommandSource::Package {
                    package_id: "pkg-one".to_string(),
                },
            },
            super::CommandDirectory {
                root: second,
                source: CommandSource::Package {
                    package_id: "pkg-two".to_string(),
                },
            },
        ])
        .unwrap_err()
        .to_string();
        assert!(err.contains("duplicate command `greet`"), "{err}");
        assert!(err.contains("pkg-one") && err.contains("pkg-two"), "{err}");
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
