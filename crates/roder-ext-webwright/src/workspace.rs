use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

use crate::artifacts::{
    WebwrightLogSummary, WebwrightPlanSummary, WebwrightScriptSummary, WebwrightSelfReflectSummary,
    read_log_summary, read_optional_self_reflect, read_plan_summary, read_script_summary,
    uses_full_page_screenshot,
};
use crate::errors::path_display;
use crate::showcase::{
    WebwrightReport, WebwrightTaskDefinition, read_report, read_task_definition,
};

pub const MANIFEST_FILE: &str = "webwright.json";
pub const PLAN_FILE: &str = "plan.md";
pub const FINAL_SCRIPT_FILE: &str = "final_script.py";
pub const FINAL_RUNS_DIR: &str = "final_runs";
pub const FINAL_LOG_FILE: &str = "final_script_log.txt";
pub const SELF_REFLECT_FILE: &str = "self_reflect_result.json";

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WebwrightMode {
    #[default]
    Run,
    Craft,
}

impl WebwrightMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Run => "run",
            Self::Craft => "craft",
        }
    }

    pub fn parse(value: &str) -> anyhow::Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "run" | "one-shot" | "oneshot" => Ok(Self::Run),
            "craft" | "cli" | "tool" => Ok(Self::Craft),
            other => bail!("unsupported Webwright mode {other:?}; expected run or craft"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WebwrightManifest {
    pub task_id: String,
    pub task: String,
    pub mode: WebwrightMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_url: Option<String>,
    pub browser: String,
    pub headless: bool,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_run: Option<u32>,
    pub verification_state: String,
}

impl WebwrightManifest {
    pub fn new(
        task_id: impl Into<String>,
        task: impl Into<String>,
        mode: WebwrightMode,
        start_url: Option<String>,
        browser: Option<String>,
        headless: bool,
    ) -> Self {
        Self {
            task_id: task_id.into(),
            task: task.into(),
            mode,
            start_url,
            browser: browser.unwrap_or_else(|| "firefox".to_string()),
            headless,
            created_at: time::OffsetDateTime::now_utc().unix_timestamp().to_string(),
            latest_run: None,
            verification_state: "pending".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WebwrightWorkspaceSummary {
    pub root: String,
    pub manifest: Option<WebwrightManifest>,
    pub plan_path: String,
    pub plan: WebwrightPlanSummary,
    pub final_script_path: String,
    pub final_script: WebwrightScriptSummary,
    pub runs: Vec<WebwrightRunSummary>,
    pub latest_run: Option<u32>,
    pub task_definition: Option<WebwrightTaskDefinition>,
    pub report: Option<WebwrightReport>,
    pub validation_errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WebwrightRunSummary {
    pub run_id: u32,
    pub run_dir: String,
    pub final_script_path: String,
    pub final_script: WebwrightScriptSummary,
    pub log_path: String,
    pub log: WebwrightLogSummary,
    pub screenshots: Vec<String>,
    pub log_tail: Vec<String>,
    pub self_reflect: Option<WebwrightSelfReflectSummary>,
}

#[derive(Debug, Clone)]
pub struct WebwrightWorkspace {
    root: PathBuf,
}

impl WebwrightWorkspace {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn create(&self, manifest: &WebwrightManifest) -> anyhow::Result<()> {
        fs::create_dir_all(self.final_runs_dir())
            .with_context(|| format!("create Webwright workspace {}", self.root.display()))?;
        self.write_manifest(manifest)?;
        Ok(())
    }

    pub fn write_manifest(&self, manifest: &WebwrightManifest) -> anyhow::Result<()> {
        fs::create_dir_all(&self.root)?;
        let path = self.root.join(MANIFEST_FILE);
        let text = serde_json::to_string_pretty(manifest)?;
        fs::write(&path, text).with_context(|| format!("write {}", path.display()))
    }

    pub fn read_manifest(&self) -> anyhow::Result<Option<WebwrightManifest>> {
        let path = self.root.join(MANIFEST_FILE);
        if !path.exists() {
            return Ok(None);
        }
        let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        serde_json::from_str(&text)
            .map(Some)
            .with_context(|| format!("parse {}", path.display()))
    }

    pub fn ensure_starter_files(&self, manifest: &WebwrightManifest) -> anyhow::Result<()> {
        fs::create_dir_all(&self.root)?;
        let plan = self.root.join(PLAN_FILE);
        if !plan.exists() {
            fs::write(&plan, starter_plan(manifest))
                .with_context(|| format!("write {}", plan.display()))?;
        }
        let script = self.root.join(FINAL_SCRIPT_FILE);
        if !script.exists() {
            fs::write(&script, starter_script(manifest))
                .with_context(|| format!("write {}", script.display()))?;
        }
        Ok(())
    }

    pub fn write_plan(&self, text: &str) -> anyhow::Result<()> {
        write_text_file(&self.root.join(PLAN_FILE), text)
    }

    pub fn write_final_script(&self, text: &str) -> anyhow::Result<()> {
        write_text_file(&self.root.join(FINAL_SCRIPT_FILE), text)
    }

    pub fn write_task_definition(&self, task: &WebwrightTaskDefinition) -> anyhow::Result<()> {
        write_json_file(&self.root.join("task.json"), task)
    }

    pub fn write_report(&self, report: &WebwrightReport) -> anyhow::Result<()> {
        write_json_file(&self.root.join("report.json"), report)
    }

    pub fn write_self_reflect_result(
        &self,
        run_id: u32,
        value: &serde_json::Value,
    ) -> anyhow::Result<()> {
        write_json_file(&self.run_dir(run_id).join(SELF_REFLECT_FILE), value)
    }

    pub fn next_run_id(&self) -> anyhow::Result<u32> {
        Ok(self
            .run_ids()?
            .into_iter()
            .max()
            .map(|id| id + 1)
            .unwrap_or(1))
    }

    pub fn latest_run_id(&self) -> anyhow::Result<Option<u32>> {
        Ok(self.run_ids()?.into_iter().max())
    }

    pub fn summary(&self) -> anyhow::Result<WebwrightWorkspaceSummary> {
        let manifest = self.read_manifest()?;
        let runs = self.run_summaries()?;
        let latest_run = runs.iter().map(|run| run.run_id).max();
        let plan_path = self.root.join(PLAN_FILE);
        let final_script_path = self.root.join(FINAL_SCRIPT_FILE);
        let task_definition = read_optional_task_definition(&self.root.join("task.json"))?;
        let report = read_optional_report(&self.root.join("report.json"))?;
        let validation_errors = self.validation_errors();
        Ok(WebwrightWorkspaceSummary {
            root: path_display(&self.root),
            manifest,
            plan_path: path_display(&plan_path),
            plan: read_plan_summary(&plan_path)?,
            final_script_path: path_display(&final_script_path),
            final_script: read_script_summary(&final_script_path)?,
            runs,
            latest_run,
            task_definition,
            report,
            validation_errors,
        })
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        let errors = self.validation_errors();
        if errors.is_empty() {
            Ok(())
        } else {
            bail!("invalid Webwright workspace: {}", errors.join("; "))
        }
    }

    pub fn resolve_inside(&self, value: impl AsRef<Path>) -> anyhow::Result<PathBuf> {
        let value = value.as_ref();
        if value.is_absolute() {
            bail!("Webwright paths must be relative to the workspace");
        }
        let mut clean = PathBuf::new();
        for component in value.components() {
            match component {
                Component::Normal(part) => clean.push(part),
                Component::CurDir => {}
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    bail!("Webwright path {:?} escapes the workspace", value)
                }
            }
        }
        Ok(self.root.join(clean))
    }

    pub fn final_runs_dir(&self) -> PathBuf {
        self.root.join(FINAL_RUNS_DIR)
    }

    pub fn run_dir(&self, run_id: u32) -> PathBuf {
        self.final_runs_dir().join(format!("run_{run_id:03}"))
    }

    pub(crate) fn run_ids(&self) -> anyhow::Result<Vec<u32>> {
        let dir = self.final_runs_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut ids = Vec::new();
        for entry in fs::read_dir(&dir).with_context(|| format!("read {}", dir.display()))? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let Some(raw) = name.strip_prefix("run_") else {
                continue;
            };
            if let Ok(id) = raw.parse::<u32>() {
                ids.push(id);
            }
        }
        ids.sort_unstable();
        Ok(ids)
    }

    fn run_summaries(&self) -> anyhow::Result<Vec<WebwrightRunSummary>> {
        self.run_ids()?
            .into_iter()
            .map(|run_id| self.run_summary(run_id))
            .collect()
    }

    fn run_summary(&self, run_id: u32) -> anyhow::Result<WebwrightRunSummary> {
        let run_dir = self.run_dir(run_id);
        let screenshots = screenshot_paths(&run_dir.join("screenshots"))?;
        let final_script_path = run_dir.join(FINAL_SCRIPT_FILE);
        let log_path = run_dir.join(FINAL_LOG_FILE);
        let log = read_log_summary(&log_path, 20)?;
        Ok(WebwrightRunSummary {
            run_id,
            run_dir: path_display(&run_dir),
            final_script_path: path_display(&final_script_path),
            final_script: read_script_summary(&final_script_path)?,
            log_path: path_display(&log_path),
            log_tail: log.tail.clone(),
            log,
            screenshots: screenshots.iter().map(|path| path_display(path)).collect(),
            self_reflect: read_optional_self_reflect(&run_dir.join(SELF_REFLECT_FILE))?,
        })
    }

    fn validation_errors(&self) -> Vec<String> {
        let mut errors = Vec::new();
        require_file(&mut errors, &self.root.join(PLAN_FILE), "missing plan.md");
        require_file(
            &mut errors,
            &self.root.join(FINAL_SCRIPT_FILE),
            "missing final_script.py",
        );
        reject_full_page_marker(&mut errors, &self.root.join(FINAL_SCRIPT_FILE));
        match self.latest_run_id() {
            Ok(Some(run_id)) => self.validate_run(run_id, &mut errors),
            Ok(None) => errors.push("missing final_runs/run_<id> directory".to_string()),
            Err(err) => errors.push(err.to_string()),
        }
        errors
    }

    fn validate_run(&self, run_id: u32, errors: &mut Vec<String>) {
        let run_dir = self.run_dir(run_id);
        require_file(
            errors,
            &run_dir.join(FINAL_SCRIPT_FILE),
            "missing run final_script.py",
        );
        require_file(
            errors,
            &run_dir.join(FINAL_LOG_FILE),
            "missing final_script_log.txt",
        );
        reject_full_page_marker(errors, &run_dir.join(FINAL_SCRIPT_FILE));
        match screenshot_paths(&run_dir.join("screenshots")) {
            Ok(paths) if paths.is_empty() => {
                errors.push("latest run has no final_execution screenshots".to_string())
            }
            Ok(_) => {}
            Err(err) => errors.push(err.to_string()),
        }
    }
}

pub fn sanitize_task_id(value: &str) -> String {
    let mut out = String::new();
    let mut previous_dash = false;
    for ch in value.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            previous_dash = false;
        } else if !previous_dash && !out.is_empty() {
            out.push('-');
            previous_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "webwright-task".to_string()
    } else {
        out.chars().take(64).collect()
    }
}

pub(crate) fn scoped_path(
    root: &Path,
    value: impl AsRef<Path>,
    label: &str,
) -> anyhow::Result<PathBuf> {
    let value = value.as_ref();
    if value
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        bail!("{label} must stay inside the workspace root");
    }
    let resolved = if value.is_absolute() {
        value.to_path_buf()
    } else {
        root.join(value)
    };
    if !resolved.starts_with(root) {
        bail!("{label} must stay inside the workspace root");
    }
    Ok(resolved)
}

fn starter_plan(manifest: &WebwrightManifest) -> String {
    format!(
        "# Critical Points\n- [ ] CP1: Complete the requested Webwright task: {}\n",
        manifest.task
    )
}

fn starter_script(manifest: &WebwrightManifest) -> String {
    format!(
        r#""""Starter Webwright final script for task {task_id}."""

def main():
    raise SystemExit("Webwright final_script.py has not been authored yet.")

if __name__ == "__main__":
    main()
"#,
        task_id = manifest.task_id
    )
}

fn require_file(errors: &mut Vec<String>, path: &Path, message: &str) {
    if !path.is_file() {
        errors.push(format!("{message}: {}", path.display()));
    }
}

fn write_text_file(path: &Path, text: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    fs::write(path, text).with_context(|| format!("write {}", path.display()))
}

fn write_json_file<T: Serialize>(path: &Path, value: &T) -> anyhow::Result<()> {
    write_text_file(path, &serde_json::to_string_pretty(value)?)
}

fn reject_full_page_marker(errors: &mut Vec<String>, path: &Path) {
    let Ok(text) = fs::read_to_string(path) else {
        return;
    };
    if uses_full_page_screenshot(&text) {
        errors.push(format!(
            "full-page screenshots are not allowed: {}",
            path.display()
        ));
    }
}

fn screenshot_paths(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    for entry in fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name.starts_with("final_execution_") && name.ends_with(".png") {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn read_optional_task_definition(path: &Path) -> anyhow::Result<Option<WebwrightTaskDefinition>> {
    if path.exists() {
        read_task_definition(path).map(Some)
    } else {
        Ok(None)
    }
}

fn read_optional_report(path: &Path) -> anyhow::Result<Option<WebwrightReport>> {
    if path.exists() {
        read_report(path).map(Some)
    } else {
        Ok(None)
    }
}
