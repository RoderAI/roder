use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use serde::Serialize;

use crate::errors::path_display;
use crate::redaction::redact_sensitive_text;
use crate::visual::VISUAL_JUDGE_DIR;
use crate::workspace::{
    FINAL_LOG_FILE, FINAL_RUNS_DIR, FINAL_SCRIPT_FILE, MANIFEST_FILE, PLAN_FILE, SELF_REFLECT_FILE,
    WebwrightWorkspace,
};

const EXPORT_MANIFEST_FILE: &str = "webwright-export.json";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WebwrightExportResult {
    pub export_dir: String,
    pub files: Vec<String>,
    pub excluded: Vec<String>,
}

pub fn export_workspace(
    workspace: &WebwrightWorkspace,
    export_dir: impl AsRef<Path>,
) -> anyhow::Result<WebwrightExportResult> {
    let export_dir = export_dir.as_ref();
    if export_dir.starts_with(workspace.root()) {
        bail!("Webwright export directory must not be inside the source workspace");
    }
    fs::create_dir_all(export_dir)
        .with_context(|| format!("create export directory {}", export_dir.display()))?;

    let mut copied = BTreeSet::new();
    let mut excluded = BTreeSet::new();
    copy_text_if_exists(
        workspace.root(),
        export_dir,
        Path::new(MANIFEST_FILE),
        &mut copied,
    )?;
    copy_text_if_exists(
        workspace.root(),
        export_dir,
        Path::new(PLAN_FILE),
        &mut copied,
    )?;
    copy_text_if_exists(
        workspace.root(),
        export_dir,
        Path::new(FINAL_SCRIPT_FILE),
        &mut copied,
    )?;
    copy_text_if_exists(
        workspace.root(),
        export_dir,
        Path::new("task.json"),
        &mut copied,
    )?;
    copy_text_if_exists(
        workspace.root(),
        export_dir,
        Path::new("report.json"),
        &mut copied,
    )?;

    for run_id in workspace.run_ids()? {
        let run_rel = PathBuf::from(FINAL_RUNS_DIR).join(format!("run_{run_id:03}"));
        copy_text_if_exists(
            workspace.root(),
            export_dir,
            &run_rel.join(FINAL_SCRIPT_FILE),
            &mut copied,
        )?;
        copy_text_if_exists(
            workspace.root(),
            export_dir,
            &run_rel.join(FINAL_LOG_FILE),
            &mut copied,
        )?;
        copy_text_if_exists(
            workspace.root(),
            export_dir,
            &run_rel.join(SELF_REFLECT_FILE),
            &mut copied,
        )?;
        copy_screenshots(workspace.root(), export_dir, &run_rel, &mut copied)?;
    }
    copy_visual_judge_records(workspace.root(), export_dir, &mut copied)?;

    collect_excluded(workspace.root(), &copied, &mut excluded)?;
    let result = WebwrightExportResult {
        export_dir: path_display(export_dir),
        files: copied.iter().cloned().collect(),
        excluded: excluded.iter().cloned().collect(),
    };
    let manifest_path = export_dir.join(EXPORT_MANIFEST_FILE);
    fs::write(&manifest_path, serde_json::to_string_pretty(&result)?)
        .with_context(|| format!("write {}", manifest_path.display()))?;
    Ok(WebwrightExportResult {
        files: {
            let mut files = result.files.clone();
            files.push(EXPORT_MANIFEST_FILE.to_string());
            files.sort();
            files
        },
        ..result
    })
}

fn copy_text_if_exists(
    root: &Path,
    export_dir: &Path,
    rel: &Path,
    copied: &mut BTreeSet<String>,
) -> anyhow::Result<()> {
    let source = root.join(rel);
    if !source.is_file() {
        return Ok(());
    }
    let text = fs::read_to_string(&source).with_context(|| format!("read {}", source.display()))?;
    write_export_file(export_dir, rel, redact_sensitive_text(&text))?;
    copied.insert(rel_display(rel));
    Ok(())
}

fn copy_screenshots(
    root: &Path,
    export_dir: &Path,
    run_rel: &Path,
    copied: &mut BTreeSet<String>,
) -> anyhow::Result<()> {
    let screenshot_rel = run_rel.join("screenshots");
    let screenshot_dir = root.join(&screenshot_rel);
    if !screenshot_dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(&screenshot_dir)
        .with_context(|| format!("read {}", screenshot_dir.display()))?
    {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !(name.starts_with("final_execution_") && name.ends_with(".png")) {
            continue;
        }
        let rel = screenshot_rel.join(name);
        let target = export_dir.join(&rel);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        fs::copy(&path, &target)
            .with_context(|| format!("copy {} to {}", path.display(), target.display()))?;
        copied.insert(rel_display(&rel));
    }
    Ok(())
}

fn copy_visual_judge_records(
    root: &Path,
    export_dir: &Path,
    copied: &mut BTreeSet<String>,
) -> anyhow::Result<()> {
    let dir = root.join(VISUAL_JUDGE_DIR);
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(&dir).with_context(|| format!("read {}", dir.display()))? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.ends_with(".json") {
            continue;
        }
        let rel = Path::new(VISUAL_JUDGE_DIR).join(name);
        copy_text_if_exists(root, export_dir, &rel, copied)?;
    }
    Ok(())
}

fn collect_excluded(
    root: &Path,
    copied: &BTreeSet<String>,
    excluded: &mut BTreeSet<String>,
) -> anyhow::Result<()> {
    if !root.exists() {
        return Ok(());
    }
    collect_excluded_inner(root, root, copied, excluded)
}

fn collect_excluded_inner(
    root: &Path,
    dir: &Path,
    copied: &BTreeSet<String>,
    excluded: &mut BTreeSet<String>,
) -> anyhow::Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_excluded_inner(root, &path, copied, excluded)?;
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .with_context(|| format!("strip {} from {}", root.display(), path.display()))?;
        let rel = rel_display(rel);
        if !copied.contains(&rel) {
            excluded.insert(rel);
        }
    }
    Ok(())
}

fn write_export_file(export_dir: &Path, rel: &Path, text: String) -> anyhow::Result<()> {
    let target = export_dir.join(rel);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    fs::write(&target, text).with_context(|| format!("write {}", target.display()))
}

fn rel_display(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}
