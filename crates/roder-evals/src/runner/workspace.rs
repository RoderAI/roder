use std::collections::BTreeSet;
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
            if let Some(exact) = &expected.exact_contents
                && contents != *exact
            {
                anyhow::bail!(
                    "{} did not exactly match expected contents",
                    expected.path.display()
                );
            }
            if let Some(max_bytes) = expected.max_bytes {
                let actual = contents.as_bytes().len() as u64;
                if actual > max_bytes {
                    anyhow::bail!(
                        "{} was {actual} bytes, exceeding maxBytes {max_bytes}",
                        expected.path.display()
                    );
                }
            }
            if let Some(allowed_chars) = &expected.allowed_chars {
                let allowed = allowed_chars.chars().collect::<BTreeSet<_>>();
                for character in contents.chars() {
                    if !allowed.contains(&character) {
                        anyhow::bail!(
                            "{} contained disallowed character {:?}",
                            expected.path.display(),
                            character
                        );
                    }
                }
            }
            if !expected.json_array_fields.is_empty() {
                let json: serde_json::Value =
                    serde_json::from_str(&contents).with_context(|| {
                        format!("{} did not contain valid JSON", expected.path.display())
                    })?;
                for field in &expected.json_array_fields {
                    let Some(value) = json_field(&json, field) else {
                        anyhow::bail!("{} missing JSON field `{field}`", expected.path.display());
                    };
                    if !value.is_array() {
                        anyhow::bail!(
                            "{} JSON field `{field}` was not an array",
                            expected.path.display()
                        );
                    }
                }
            }
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
    #[cfg(windows)]
    let mut shell = {
        let mut command_process = Command::new("cmd");
        command_process.arg("/C").arg(command);
        command_process
    };
    #[cfg(not(windows))]
    let mut shell = {
        let mut command_process = Command::new("sh");
        command_process.arg("-c").arg(command);
        command_process
    };

    shell
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

fn json_field<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    path.split('.').try_fold(value, |current, part| {
        current.as_object().and_then(|object| object.get(part))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expected_file_contract_accepts_exact_json_array_size_and_allowed_chars() {
        let root =
            std::env::temp_dir().join(format!("roder-tbench-contract-ok-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("out.txt"), "flag{gc0d3_iz_ch4LLenGiNg}\n").unwrap();
        std::fs::write(
            root.join("sam.json"),
            r#"{"coords_x":[1,2],"coords_y":[3,4]}"#,
        )
        .unwrap();
        std::fs::write(root.join("gblock.txt"), "ACGTACGT\n").unwrap();
        let fixture: EvalFixture = serde_json::from_value(serde_json::json!({
            "id": "tbench-contract-ok",
            "title": "TBench exact file contract",
            "prompt": "Check the output contracts.",
            "expected": {
                "files": [
                    {
                        "path": "out.txt",
                        "exactContents": "flag{gc0d3_iz_ch4LLenGiNg}\n"
                    },
                    {
                        "path": "sam.json",
                        "jsonArrayFields": ["coords_x", "coords_y"]
                    },
                    {
                        "path": "gblock.txt",
                        "maxBytes": 3000,
                        "allowedChars": "ACGT\n"
                    }
                ]
            }
        }))
        .unwrap();

        let result = grade_expected_evidence(&fixture, &root, "done");

        assert!(result.is_ok(), "{result:?}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn expected_file_contract_rejects_non_array_json_field() {
        let root = std::env::temp_dir().join(format!(
            "roder-tbench-contract-bad-json-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("sam.json"), r#"{"coords_x":"(1, 2)"}"#).unwrap();
        let fixture: EvalFixture = serde_json::from_value(serde_json::json!({
            "id": "tbench-contract-bad-json",
            "title": "TBench bad JSON contract",
            "prompt": "Check the output contracts.",
            "expected": {
                "files": [
                    {
                        "path": "sam.json",
                        "jsonArrayFields": ["coords_x"]
                    }
                ]
            }
        }))
        .unwrap();

        let error = grade_expected_evidence(&fixture, &root, "done").unwrap_err();

        assert!(
            error
                .to_string()
                .contains("sam.json JSON field `coords_x` was not an array"),
            "{error}"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn expected_file_contract_rejects_overlong_or_disallowed_sequence() {
        let root = std::env::temp_dir().join(format!(
            "roder-tbench-contract-bad-sequence-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("gblock.txt"), "ACGTNN\n").unwrap();
        let fixture: EvalFixture = serde_json::from_value(serde_json::json!({
            "id": "tbench-contract-bad-sequence",
            "title": "TBench bad sequence contract",
            "prompt": "Check the output contracts.",
            "expected": {
                "files": [
                    {
                        "path": "gblock.txt",
                        "maxBytes": 4,
                        "allowedChars": "ACGT\n"
                    }
                ]
            }
        }))
        .unwrap();

        let error = grade_expected_evidence(&fixture, &root, "done").unwrap_err();

        assert!(
            error
                .to_string()
                .contains("gblock.txt was 7 bytes, exceeding maxBytes 4")
                || error
                    .to_string()
                    .contains("gblock.txt contained disallowed character"),
            "{error}"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
