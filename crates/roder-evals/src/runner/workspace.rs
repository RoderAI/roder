use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context;

use crate::{EvalFailureClass, EvalFixture};

pub(super) struct EvalWorkspace {
    pub path: PathBuf,
}

impl Drop for EvalWorkspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

pub(super) fn create_workspace(fixture: &EvalFixture) -> anyhow::Result<EvalWorkspace> {
    let path = std::env::temp_dir().join(format!(
        "roder-eval-{}-{}",
        fixture.id,
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&path)?;
    for file in &fixture.workspace.files {
        let file_path = safe_workspace_path(&path, &file.path)?;
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(file_path, &file.contents)?;
    }
    Ok(EvalWorkspace { path })
}

pub(super) fn run_workspace_setup(fixture: &EvalFixture, workspace: &Path) -> anyhow::Result<()> {
    for command in &fixture.workspace.commands {
        let output = run_shell_command(command, workspace)?;
        if !output.status.success() {
            anyhow::bail!(
                "setup command `{}` failed with status {:?}",
                command,
                output.status.code()
            );
        }
    }
    Ok(())
}

pub(super) fn grade_expected_evidence(
    fixture: &EvalFixture,
    workspace: &Path,
    final_answer: &str,
) -> anyhow::Result<()> {
    for needle in &fixture.expected.final_answer_contains {
        if !final_answer.contains(needle) {
            anyhow::bail!("final answer did not contain `{needle}`");
        }
    }
    for expected in &fixture.expected.files {
        let path = safe_workspace_path(workspace, &expected.path)?;
        if expected.exists && !path.exists() {
            anyhow::bail!("expected file missing: {}", expected.path.display());
        }
        if !expected.exists && path.exists() {
            anyhow::bail!("file should not exist: {}", expected.path.display());
        }
        if path.exists() {
            let contents = std::fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", expected.path.display()))?;
            for needle in &expected.contains {
                if !contents.contains(needle) {
                    anyhow::bail!("{} did not contain `{needle}`", expected.path.display());
                }
            }
        }
    }
    for check in &fixture.expected.command_checks {
        let output = run_shell_command(&check.command, workspace)?;
        let code = output.status.code().unwrap_or(-1);
        if code != check.expected_exit_code {
            anyhow::bail!(
                "command `{}` exited {code}, expected {}",
                check.command,
                check.expected_exit_code
            );
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        for needle in &check.stdout_contains {
            if !stdout.contains(needle) {
                anyhow::bail!("command `{}` stdout missing `{needle}`", check.command);
            }
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        for needle in &check.stderr_contains {
            if !stderr.contains(needle) {
                anyhow::bail!("command `{}` stderr missing `{needle}`", check.command);
            }
        }
    }
    Ok(())
}

pub(super) fn failure_class_for_fixture(fixture: &EvalFixture) -> EvalFailureClass {
    if fixture.tags.iter().any(|tag| tag == "tool-misuse") {
        EvalFailureClass::ToolSchema
    } else if fixture
        .tags
        .iter()
        .any(|tag| tag == "verification-before-final")
    {
        EvalFailureClass::Verifier
    } else {
        EvalFailureClass::Model
    }
}

fn run_shell_command(command: &str, cwd: &Path) -> anyhow::Result<std::process::Output> {
    Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .output()
        .with_context(|| format!("failed to run `{command}`"))
}

fn safe_workspace_path(root: &Path, relative: &Path) -> anyhow::Result<PathBuf> {
    if relative.is_absolute()
        || relative
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        anyhow::bail!("workspace paths must be relative and stay inside the temp workspace");
    }
    Ok(root.join(relative))
}
