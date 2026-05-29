use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use roder_commands::{
    CommandDirectory, CommandSource, CommandSpec, CommandsRegistry, CommandsRegistryOptions,
    WorkflowCommandDirectory,
};

pub(super) fn run_commands_cli(args: &[String], cfg: &roder_config::Config) -> anyhow::Result<()> {
    match args.first().map(String::as_str) {
        Some("list") => {
            let registry = load_registry(cfg)?;
            print_commands_list(&registry);
        }
        Some("show") => {
            let Some(name) = args.get(1) else {
                anyhow::bail!("roder commands show requires a command name");
            };
            let registry = load_registry(cfg)?;
            let spec = registry
                .get(name)
                .ok_or_else(|| anyhow::anyhow!("unknown command {name:?}"))?;
            print_command_show(spec);
        }
        _ => anyhow::bail!("usage: roder commands <list|show NAME>"),
    }
    Ok(())
}

fn load_registry(cfg: &roder_config::Config) -> anyhow::Result<CommandsRegistry> {
    let command_cfg = cfg.commands.clone().unwrap_or_default();
    if !command_cfg.enabled {
        anyhow::bail!("commands are disabled by configuration");
    }
    let user_dir = resolve_user_command_dir(&command_cfg);
    let workspace_dir = resolve_workspace_command_dir(&command_cfg)?;
    let workflow_workspace = std::env::current_dir().ok();
    let workflow_dirs_config = roder_config::dynamic_workflows::resolve_workflow_directories(
        cfg.dynamic_workflows.as_ref(),
        workflow_workspace.as_deref(),
    );
    let user_workflows_dir = Some(workflow_dirs_config.user);
    let workspace_workflows_dir = Some(workflow_dirs_config.workspace);
    if command_cfg.live_reload {
        let roots = [
            user_dir.clone(),
            workspace_dir.clone(),
            user_workflows_dir.clone(),
            workspace_workflows_dir.clone(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        let mut watcher =
            CommandRegistryWatcher::new(roots, Duration::from_millis(250), SystemCommandClock)?;
        let _ = watcher.poll()?;
    }
    let command_dirs = command_directories(user_dir, workspace_dir);
    let workflow_dirs = workflow_command_directories(user_workflows_dir, workspace_workflows_dir);
    CommandsRegistry::from_directories_with_workflows(
        command_dirs,
        workflow_dirs,
        CommandsRegistryOptions {
            include_builtins: true,
            allow_builtin_override: false,
        },
    )
}

fn print_commands_list(registry: &CommandsRegistry) {
    for (_, spec) in registry.iter() {
        let description = spec.description.as_deref().unwrap_or("");
        println!(
            "{:<24} {:<14} {}",
            format!("/{}", spec.name),
            spec.display_source(),
            description
        );
    }
}

fn print_command_show(spec: &CommandSpec) {
    println!("---");
    println!("name: {}", spec.name);
    if let Some(description) = &spec.description {
        println!("description: {description}");
    }
    if let Some(argument_hint) = &spec.argument_hint {
        println!("argument-hint: {argument_hint}");
    }
    if !spec.allowed_tools.is_empty() {
        println!("allowed-tools: [{}]", spec.allowed_tools.join(", "));
    }
    if let Some(model) = &spec.model {
        println!("model: {model}");
    }
    if let Some(agent) = &spec.agent {
        println!("agent: {agent}");
    }
    println!("source: {}", spec.display_source());
    if let Some(path) = &spec.path {
        println!("path: {}", path.display());
    }
    if let Some(workflow) = &spec.workflow {
        println!("workflow: true");
        println!("workflow-script-id: {}", workflow.script_id);
        println!("workflow-script-hash: {}", workflow.script_hash);
    }
    println!("---");
    println!("{}", spec.body);
}

fn command_directories(
    user_dir: Option<PathBuf>,
    workspace_dir: Option<PathBuf>,
) -> Vec<CommandDirectory> {
    let mut directories = Vec::new();
    if let Some(root) = user_dir {
        directories.push(CommandDirectory {
            root,
            source: CommandSource::User,
        });
    }
    if let Some(root) = workspace_dir {
        directories.push(CommandDirectory {
            root,
            source: CommandSource::Workspace,
        });
    }
    directories
}

fn workflow_command_directories(
    user_dir: Option<PathBuf>,
    workspace_dir: Option<PathBuf>,
) -> Vec<WorkflowCommandDirectory> {
    let mut directories = Vec::new();
    if let Some(root) = user_dir {
        directories.push(WorkflowCommandDirectory {
            root,
            source: CommandSource::User,
        });
    }
    if let Some(root) = workspace_dir {
        directories.push(WorkflowCommandDirectory {
            root,
            source: CommandSource::Workspace,
        });
    }
    directories
}

fn resolve_user_command_dir(cfg: &roder_config::CommandsConfig) -> Option<PathBuf> {
    cfg.user_dir
        .as_deref()
        .map(expand_tilde)
        .or_else(default_user_command_dir)
}

fn resolve_workspace_command_dir(
    cfg: &roder_config::CommandsConfig,
) -> anyhow::Result<Option<PathBuf>> {
    if let Some(path) = cfg.workspace_dir.as_deref() {
        return Ok(Some(expand_tilde(path)));
    }
    Ok(None)
}

fn default_user_command_dir() -> Option<PathBuf> {
    Some(roder_config::config_dir().join("commands"))
}

fn expand_tilde(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if text == "~" {
        home_dir().unwrap_or_else(|| path.to_path_buf())
    } else if let Some(rest) = text.strip_prefix("~/") {
        home_dir()
            .map(|home| home.join(rest))
            .unwrap_or_else(|| path.to_path_buf())
    } else {
        path.to_path_buf()
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

pub(super) trait CommandClock {
    fn now(&self) -> SystemTime;
}

#[derive(Debug, Clone, Copy)]
pub(super) struct SystemCommandClock;

impl CommandClock for SystemCommandClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}

#[derive(Debug, Clone)]
pub(super) struct CommandRegistryWatcher<C> {
    roots: Vec<PathBuf>,
    debounce: Duration,
    clock: C,
    snapshot: BTreeMap<PathBuf, Option<FileSnapshot>>,
    changed_at: Option<SystemTime>,
}

impl<C: CommandClock> CommandRegistryWatcher<C> {
    pub(super) fn new(roots: Vec<PathBuf>, debounce: Duration, clock: C) -> anyhow::Result<Self> {
        let snapshot = snapshot_roots(&roots)?;
        Ok(Self {
            roots,
            debounce,
            clock,
            snapshot,
            changed_at: None,
        })
    }

    pub(super) fn poll(&mut self) -> anyhow::Result<bool> {
        let next = snapshot_roots(&self.roots)?;
        let now = self.clock.now();
        if next != self.snapshot {
            self.snapshot = next;
            if self.changed_at.is_none() {
                self.changed_at = Some(now);
            }
        }
        let Some(changed_at) = self.changed_at else {
            return Ok(false);
        };
        if now.duration_since(changed_at).unwrap_or_default() >= self.debounce {
            self.changed_at = None;
            return Ok(true);
        }
        Ok(false)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileSnapshot {
    modified: Option<SystemTime>,
    len: u64,
}

fn snapshot_roots(roots: &[PathBuf]) -> anyhow::Result<BTreeMap<PathBuf, Option<FileSnapshot>>> {
    let mut snapshot = BTreeMap::new();
    for root in roots {
        snapshot_dir(root, &mut snapshot)?;
    }
    Ok(snapshot)
}

fn snapshot_dir(
    root: &Path,
    snapshot: &mut BTreeMap<PathBuf, Option<FileSnapshot>>,
) -> anyhow::Result<()> {
    if !root.exists() {
        snapshot.insert(root.to_path_buf(), None);
        return Ok(());
    }
    let metadata = root.metadata()?;
    snapshot.insert(
        root.to_path_buf(),
        Some(FileSnapshot {
            modified: metadata.modified().ok(),
            len: metadata.len(),
        }),
    );
    if root.is_dir() {
        let mut entries = std::fs::read_dir(root)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| entry.path());
        for entry in entries {
            snapshot_dir(&entry.path(), snapshot)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        rc::Rc,
        time::{Duration, UNIX_EPOCH},
    };

    use super::*;

    #[derive(Clone)]
    struct FakeClock {
        millis: Rc<std::cell::Cell<u64>>,
    }

    impl CommandClock for FakeClock {
        fn now(&self) -> SystemTime {
            UNIX_EPOCH + Duration::from_millis(self.millis.get())
        }
    }

    #[test]
    fn commands_registry_loads_workspace_overrides() {
        let root = temp_root("commands_registry_loads_workspace_overrides");
        let user = root.join("user");
        let workspace = root.join("workspace");
        write(&user.join("review.md"), "---\ndescription: user\n---\nUser");
        write(
            &workspace.join("review.md"),
            "---\ndescription: workspace\n---\nWorkspace",
        );
        let cfg = roder_config::Config {
            commands: Some(roder_config::CommandsConfig {
                user_dir: Some(user),
                workspace_dir: Some(workspace),
                ..roder_config::CommandsConfig::default()
            }),
            ..roder_config::Config::default()
        };

        let registry = load_registry(&cfg).unwrap();
        assert_eq!(
            registry.get("review").unwrap().description.as_deref(),
            Some("workspace")
        );
    }

    #[test]
    fn commands_do_not_default_to_workspace_roder_dir() {
        let cfg = roder_config::CommandsConfig {
            workspace_dir: None,
            ..roder_config::CommandsConfig::default()
        };

        assert!(resolve_workspace_command_dir(&cfg).unwrap().is_none());
    }

    #[test]
    fn commands_watcher_debounces_add_edit_delete() {
        let root = temp_root("commands_watcher_debounces_add_edit_delete");
        fs::create_dir_all(&root).unwrap();
        let clock = FakeClock {
            millis: Rc::new(std::cell::Cell::new(0)),
        };
        let mut watcher = CommandRegistryWatcher::new(
            vec![root.clone()],
            Duration::from_millis(50),
            clock.clone(),
        )
        .unwrap();

        write(&root.join("review.md"), "---\n---\nReview");
        assert_reloads_after_debounce(&mut watcher, &clock, 0);

        write(&root.join("review.md"), "---\n---\nReview again");
        assert_reloads_after_debounce(&mut watcher, &clock, 100);

        fs::remove_file(root.join("review.md")).unwrap();
        assert_reloads_after_debounce(&mut watcher, &clock, 200);
    }

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("roder-cli-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn write(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn assert_reloads_after_debounce(
        watcher: &mut CommandRegistryWatcher<FakeClock>,
        clock: &FakeClock,
        base: u64,
    ) {
        clock.millis.set(base);
        assert!(!watcher.poll().unwrap());
        for offset in [25, 51, 75, 100] {
            clock.millis.set(base + offset);
            if watcher.poll().unwrap() {
                return;
            }
        }
        panic!("watcher did not reload after debounce");
    }
}
