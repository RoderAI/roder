use roder_api::workflow::{
    WorkflowImportError, WorkflowImportItem, WorkflowImportRisk, WorkflowImportScan,
    WorkflowImportState, WorkflowSource, WorkflowSourceType,
};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use time::OffsetDateTime;

#[derive(Debug, Clone)]
pub struct WorkflowScanOptions {
    pub workspace: PathBuf,
    pub include_user: bool,
    pub user_roots: Vec<PathBuf>,
}

impl WorkflowScanOptions {
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self {
            workspace: workspace.into(),
            include_user: false,
            user_roots: Vec::new(),
        }
    }
}

pub fn scan_workflow_imports(options: WorkflowScanOptions) -> WorkflowImportScan {
    let workspace = options.workspace;
    let detected_at = OffsetDateTime::now_utc();
    let mut scanner = Scanner {
        workspace: workspace.clone(),
        detected_at,
        items: Vec::new(),
        errors: Vec::new(),
    };

    scanner.scan_repo_root();
    scanner.scan_skills(&workspace.join(".agents").join("skills"));
    scanner.scan_slash_commands(&workspace.join(".agents").join("commands"));
    scanner.scan_slash_commands(&workspace.join(".claude").join("commands"));
    scanner.scan_slash_commands(&workspace.join(".roder").join("commands"));
    scanner.scan_mcp_config(&workspace.join(".mcp.json"));
    scanner.scan_mcp_config(&workspace.join(".cursor").join("mcp.json"));
    scanner.scan_mcp_config(&workspace.join("mcp.toml"));
    scanner.scan_mcp_config(&workspace.join("mcp.yaml"));
    scanner.scan_hook_config(&workspace.join(".codex").join("hooks.json"));
    scanner.scan_hook_config(&workspace.join(".cursor").join("hooks.json"));
    scanner.scan_plugins(&workspace.join(".agents").join("plugins"));
    scanner.scan_plugin_manifest(&workspace.join(".codex-plugin").join("plugin.json"));

    if options.include_user {
        for root in options.user_roots {
            scanner.scan_skills(&root.join("skills"));
            scanner.scan_slash_commands(&root.join("commands"));
            scanner.scan_mcp_config(&root.join("mcp.json"));
        }
    }

    scanner.items.sort_by(|left, right| left.id.cmp(&right.id));
    WorkflowImportScan {
        workspace: workspace.display().to_string(),
        items: scanner.items,
        errors: scanner.errors,
    }
}

struct Scanner {
    workspace: PathBuf,
    detected_at: OffsetDateTime,
    items: Vec<WorkflowImportItem>,
    errors: Vec<WorkflowImportError>,
}

impl Scanner {
    fn scan_repo_root(&mut self) {
        for name in ["AGENTS.md", "CLAUDE.md"] {
            let path = self.workspace.join(name);
            if path.exists() {
                self.push_markdown_guidance(&path, name);
            }
        }
        let readme = self.workspace.join("README.md");
        if let Ok(text) = fs::read_to_string(&readme)
            && text.to_ascii_lowercase().contains("agent")
        {
            self.push_item(
                &readme,
                WorkflowSourceType::Guidance,
                "README agent guidance",
                "Repository README contains agent-facing workflow guidance.",
                WorkflowImportRisk::Passive,
                false,
                serde_json::json!({ "excerpt": truncate(&text, 500) }),
            );
        }
    }

    fn push_markdown_guidance(&mut self, path: &Path, title: &str) {
        match fs::read_to_string(path) {
            Ok(text) => self.push_item(
                path,
                WorkflowSourceType::Guidance,
                title,
                "Repository guidance imported as passive context.",
                WorkflowImportRisk::Passive,
                false,
                serde_json::json!({ "markdown": truncate(&text, 1_200) }),
            ),
            Err(err) => self.push_error(path, err),
        }
    }

    fn scan_skills(&mut self, root: &Path) {
        let Ok(entries) = fs::read_dir(root) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path().join("SKILL.md");
            if !path.exists() {
                continue;
            }
            match fs::read_to_string(&path) {
                Ok(text) => {
                    let title = front_matter_value(&text, "name")
                        .or_else(|| entry.file_name().to_str().map(ToOwned::to_owned))
                        .unwrap_or_else(|| "skill".to_string());
                    let description =
                        front_matter_value(&text, "description").unwrap_or_else(|| {
                            "Skill instructions imported as passive context.".to_string()
                        });
                    self.push_item(
                        &path,
                        WorkflowSourceType::Skill,
                        &title,
                        &description,
                        WorkflowImportRisk::Passive,
                        false,
                        serde_json::json!({
                            "name": title,
                            "description": description,
                            "markdown": truncate(&text, 1_200),
                        }),
                    );
                }
                Err(err) => self.push_error(&path, err),
            }
        }
    }

    fn scan_slash_commands(&mut self, root: &Path) {
        let Ok(entries) = fs::read_dir(root) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
                continue;
            }
            match fs::read_to_string(&path) {
                Ok(text) => {
                    let command = path
                        .file_stem()
                        .and_then(|stem| stem.to_str())
                        .unwrap_or("command")
                        .to_string();
                    self.push_item(
                        &path,
                        WorkflowSourceType::SlashCommand,
                        &format!("/{command}"),
                        "Slash command imported for preview before use.",
                        WorkflowImportRisk::Passive,
                        false,
                        serde_json::json!({
                            "name": command,
                            "markdown": truncate(&text, 1_200),
                        }),
                    );
                }
                Err(err) => self.push_error(&path, err),
            }
        }
    }

    fn scan_mcp_config(&mut self, path: &Path) {
        if !path.exists() {
            return;
        }
        match fs::read_to_string(path) {
            Ok(text) => match parse_structured(path, &text) {
                Ok(value) => {
                    let servers = value
                        .get("mcpServers")
                        .or_else(|| value.get("servers"))
                        .and_then(serde_json::Value::as_object);
                    let Some(servers) = servers else {
                        self.push_item(
                            path,
                            WorkflowSourceType::McpServer,
                            "MCP config",
                            "MCP config file detected without mcpServers entries.",
                            WorkflowImportRisk::Unknown,
                            true,
                            redact_json(value),
                        );
                        return;
                    };
                    for (name, server) in servers {
                        let preview = redact_json(server.clone());
                        let command_capable = server.get("command").is_some()
                            || server.get("args").is_some()
                            || server.get("transport").is_some();
                        self.push_named_item(
                            path,
                            WorkflowSourceType::McpServer,
                            name,
                            &format!("MCP server {name}"),
                            "MCP server import remains disabled until explicitly enabled.",
                            WorkflowImportRisk::StartsProcess,
                            command_capable,
                            preview,
                        );
                    }
                }
                Err(message) => self.errors.push(WorkflowImportError {
                    path: self.display_path(path),
                    message,
                }),
            },
            Err(err) => self.push_error(path, err),
        }
    }

    fn scan_hook_config(&mut self, path: &Path) {
        if !path.exists() {
            return;
        }
        match fs::read_to_string(path) {
            Ok(text) => match parse_structured(path, &text) {
                Ok(value) => self.push_item(
                    path,
                    WorkflowSourceType::Hook,
                    "Hook config",
                    "Hook config detected; execution remains policy-controlled.",
                    WorkflowImportRisk::RunsHook,
                    true,
                    redact_json(value),
                ),
                Err(message) => self.errors.push(WorkflowImportError {
                    path: self.display_path(path),
                    message,
                }),
            },
            Err(err) => self.push_error(path, err),
        }
    }

    fn scan_plugins(&mut self, root: &Path) {
        let Ok(entries) = fs::read_dir(root) else {
            return;
        };
        for entry in entries.flatten() {
            self.scan_plugin_manifest(&entry.path().join("plugin.json"));
        }
    }

    fn scan_plugin_manifest(&mut self, path: &Path) {
        if !path.exists() {
            return;
        }
        match fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<serde_json::Value>(&text) {
                Ok(value) => {
                    let name = value
                        .get("name")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("plugin")
                        .to_string();
                    self.push_named_item(
                        path,
                        WorkflowSourceType::Plugin,
                        &name,
                        &format!("Plugin {name}"),
                        "Plugin manifest detected; executable capabilities stay approval-gated.",
                        WorkflowImportRisk::Unknown,
                        true,
                        redact_json(value),
                    );
                }
                Err(err) => self.errors.push(WorkflowImportError {
                    path: self.display_path(path),
                    message: err.to_string(),
                }),
            },
            Err(err) => self.push_error(path, err),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn push_named_item(
        &mut self,
        path: &Path,
        source_type: WorkflowSourceType,
        source_name: &str,
        title: &str,
        summary: &str,
        risk: WorkflowImportRisk,
        command_capable: bool,
        preview: serde_json::Value,
    ) {
        let mut item = self.build_item(path, source_type, title, summary, risk, command_capable);
        item.source.name = Some(source_name.to_string());
        item.preview = preview;
        self.items.push(item);
    }

    #[allow(clippy::too_many_arguments)]
    fn push_item(
        &mut self,
        path: &Path,
        source_type: WorkflowSourceType,
        title: &str,
        summary: &str,
        risk: WorkflowImportRisk,
        command_capable: bool,
        preview: serde_json::Value,
    ) {
        let mut item = self.build_item(path, source_type, title, summary, risk, command_capable);
        item.preview = preview;
        self.items.push(item);
    }

    fn build_item(
        &self,
        path: &Path,
        source_type: WorkflowSourceType,
        title: &str,
        summary: &str,
        risk: WorkflowImportRisk,
        command_capable: bool,
    ) -> WorkflowImportItem {
        let hash = file_hash(path).unwrap_or_else(|_| "unreadable".to_string());
        let display_path = self.display_path(path);
        let id = import_id(&source_type, &display_path, &hash);
        WorkflowImportItem {
            id,
            title: title.to_string(),
            summary: summary.to_string(),
            source: WorkflowSource {
                source_type,
                path: display_path,
                name: None,
                hash,
                detected_at: self.detected_at,
            },
            state: WorkflowImportState::Detected,
            risk,
            command_capable,
            approval_required: command_capable,
            preview: serde_json::Value::Null,
            conflicts: Vec::new(),
            enabled_at: None,
        }
    }

    fn push_error(&mut self, path: &Path, err: impl std::fmt::Display) {
        self.errors.push(WorkflowImportError {
            path: self.display_path(path),
            message: err.to_string(),
        });
    }

    fn display_path(&self, path: &Path) -> String {
        path.strip_prefix(&self.workspace)
            .unwrap_or(path)
            .display()
            .to_string()
    }
}

fn parse_structured(path: &Path, text: &str) -> Result<serde_json::Value, String> {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("toml") => toml::from_str::<serde_json::Value>(text).map_err(|err| err.to_string()),
        Some("yaml") | Some("yml") => {
            serde_yaml::from_str::<serde_json::Value>(text).map_err(|err| err.to_string())
        }
        _ => serde_json::from_str::<serde_json::Value>(text).map_err(|err| err.to_string()),
    }
}

fn file_hash(path: &Path) -> std::io::Result<String> {
    let bytes = fs::read(path)?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Ok(hex_digest(hasher.finalize()))
}

fn import_id(source_type: &WorkflowSourceType, path: &str, hash: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("{source_type:?}:{path}:{hash}"));
    let digest = hex_digest(hasher.finalize());
    format!("workflow-{}", &digest[..16])
}

fn hex_digest(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn front_matter_value(text: &str, key: &str) -> Option<String> {
    let mut lines = text.lines();
    if lines.next()? != "---" {
        return None;
    }
    for line in lines {
        if line == "---" {
            break;
        }
        let Some((found, value)) = line.split_once(':') else {
            continue;
        };
        if found.trim() == key {
            return Some(value.trim().trim_matches('"').to_string());
        }
    }
    None
}

fn truncate(text: &str, max: usize) -> String {
    let mut out = text.chars().take(max).collect::<String>();
    if text.chars().count() > max {
        out.push_str("\n...");
    }
    out
}

fn redact_json(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.into_iter()
                .map(|(key, value)| {
                    if is_secret_key(&key) {
                        (key, serde_json::json!("[redacted]"))
                    } else {
                        (key, redact_json(value))
                    }
                })
                .collect(),
        ),
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.into_iter().map(redact_json).collect())
        }
        other => other,
    }
}

fn is_secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("token")
        || key.contains("secret")
        || key.contains("password")
        || key.contains("api_key")
        || key.contains("apikey")
        || key == "env"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_import_scanner_finds_repo_conventions_and_redacts_secrets() {
        let repo = fixture_dir("scan");
        fs::create_dir_all(repo.join(".agents/skills/demo")).unwrap();
        fs::create_dir_all(repo.join(".claude/commands")).unwrap();
        fs::create_dir_all(repo.join(".roder/commands")).unwrap();
        fs::create_dir_all(repo.join(".cursor")).unwrap();
        fs::create_dir_all(repo.join(".codex")).unwrap();
        fs::create_dir_all(repo.join(".codex-plugin")).unwrap();
        fs::write(repo.join("AGENTS.md"), "Use repo conventions.").unwrap();
        fs::write(
            repo.join(".agents/skills/demo/SKILL.md"),
            "---\nname: demo\ndescription: Demo skill\n---\nBody",
        )
        .unwrap();
        fs::write(repo.join(".claude/commands/build.md"), "Run build").unwrap();
        fs::write(repo.join(".roder/commands/test.md"), "Run tests").unwrap();
        fs::write(repo.join(".codex/hooks.json"), r#"{"hooks":["cargo fmt"]}"#).unwrap();
        fs::write(
            repo.join(".codex-plugin/plugin.json"),
            r#"{"name":"demo-plugin"}"#,
        )
        .unwrap();
        fs::write(
            repo.join(".cursor/mcp.json"),
            r#"{"mcpServers":{"local":{"command":"node","env":{"API_KEY":"secret"}}}}"#,
        )
        .unwrap();

        let scan = scan_workflow_imports(WorkflowScanOptions::new(&repo));

        assert!(scan.errors.is_empty());
        assert!(scan.items.iter().any(|item| item.title == "AGENTS.md"));
        assert!(
            scan.items
                .iter()
                .any(|item| item.source.source_type == WorkflowSourceType::Skill)
        );
        assert!(
            scan.items
                .iter()
                .any(|item| item.source.source_type == WorkflowSourceType::SlashCommand)
        );
        assert!(
            scan.items
                .iter()
                .any(|item| item.source.source_type == WorkflowSourceType::Hook)
        );
        assert!(
            scan.items
                .iter()
                .any(|item| item.source.source_type == WorkflowSourceType::Plugin)
        );
        let mcp = scan
            .items
            .iter()
            .find(|item| item.source.source_type == WorkflowSourceType::McpServer)
            .unwrap();
        assert!(mcp.command_capable);
        assert!(mcp.approval_required);
        assert_eq!(mcp.preview["env"], "[redacted]");
        assert!(!repo.join(".roder/workflow-imports.json").exists());
    }

    #[test]
    fn invalid_config_reports_error_without_panicking() {
        let repo = fixture_dir("invalid");
        fs::write(repo.join(".mcp.json"), "{ nope").unwrap();

        let scan = scan_workflow_imports(WorkflowScanOptions::new(&repo));

        assert_eq!(scan.items.len(), 0);
        assert_eq!(scan.errors.len(), 1);
        assert!(scan.errors[0].path.ends_with(".mcp.json"));
    }

    fn fixture_dir(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "roder-workflow-import-{name}-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }
}
