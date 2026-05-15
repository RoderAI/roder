use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};

use crate::{
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandsRegistry {
    commands: BTreeMap<String, CommandSpec>,
}

impl CommandsRegistry {
    pub fn load(
        user_dir: Option<impl AsRef<Path>>,
        workspace_dir: Option<impl AsRef<Path>>,
        extension_dirs: impl IntoIterator<Item = ExtensionCommandDirectory>,
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
        Self::from_directories(directories)
    }

    pub fn from_directories(
        directories: impl IntoIterator<Item = CommandDirectory>,
    ) -> Result<Self> {
        let mut registry = CommandsRegistry::default();
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
