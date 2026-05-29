use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Instant;

use anyhow::Context;
use serde::Serialize;
use serde_json::Value;
use tokio::process::Command;
use tokio::time::{Duration, timeout};

use crate::types::ZerolangConfig;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ZeroCommandOutput {
    pub binary: String,
    pub argv: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    pub stdout: String,
    pub stderr: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<i32>,
    pub elapsed_ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json: Option<Value>,
}

impl ZeroCommandOutput {
    pub fn success(&self) -> bool {
        self.status == Some(0)
    }
}

#[derive(Debug, Clone)]
pub struct ZeroCommandRunner {
    config: ZerolangConfig,
}

impl ZeroCommandRunner {
    pub fn new(config: ZerolangConfig) -> Self {
        Self { config }
    }

    pub fn binary_path(&self) -> PathBuf {
        std::env::var_os("RODER_ZERO_BIN")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| self.config.binary.clone())
            .unwrap_or_else(|| PathBuf::from("zero"))
    }

    pub async fn run(
        &self,
        args: &[String],
        cwd: Option<&Path>,
        parse_json: bool,
    ) -> anyhow::Result<ZeroCommandOutput> {
        let binary = self.binary_path();
        let timeout_seconds = self.config.timeout_seconds();
        let started = Instant::now();
        let mut command = Command::new(&binary);
        command.args(args);
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        }
        command.stdin(Stdio::null());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        let child = command.spawn().with_context(|| {
            format!(
                "failed to launch Zero binary {}; set RODER_ZERO_BIN or [zerolang].binary",
                binary.display()
            )
        })?;
        let output = timeout(
            Duration::from_secs(timeout_seconds),
            child.wait_with_output(),
        )
        .await
        .with_context(|| {
            format!(
                "zero command timed out after {timeout_seconds}s: {} {}",
                binary.display(),
                args.join(" ")
            )
        })?
        .with_context(|| format!("zero command failed: {}", binary.display()))?;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let json = if parse_json && !stdout.trim().is_empty() {
            serde_json::from_str(stdout.trim()).ok()
        } else {
            None
        };
        Ok(ZeroCommandOutput {
            binary: binary.display().to_string(),
            argv: args.to_vec(),
            cwd: cwd.map(|path| path.display().to_string()),
            stdout,
            stderr,
            status: output.status.code(),
            elapsed_ms: started.elapsed().as_millis(),
            json,
        })
    }
}

impl Default for ZeroCommandRunner {
    fn default() -> Self {
        Self::new(ZerolangConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn fake_zero(name: &str, body: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "roder-zero-command-{name}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("zero");
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).unwrap();
        path
    }

    #[tokio::test]
    async fn runner_prefers_configured_binary_and_parses_json() {
        let binary = fake_zero("json", "printf '{\"ok\":true,\"argv\":\"%s\"}\\n' \"$*\"");
        let runner = ZeroCommandRunner::new(ZerolangConfig {
            binary: Some(binary.clone()),
            timeout_seconds: Some(5),
            artifact_dir: None,
        });

        let result = runner
            .run(&["doctor".to_string(), "--json".to_string()], None, true)
            .await
            .unwrap();

        assert_eq!(result.status, Some(0));
        assert_eq!(result.binary, binary.display().to_string());
        assert_eq!(result.json.unwrap()["ok"], true);
    }

    #[tokio::test]
    async fn runner_captures_non_zero_diagnostics() {
        let binary = fake_zero("nonzero", "printf 'bad input\\n' >&2\nexit 7");
        let runner = ZeroCommandRunner::new(ZerolangConfig {
            binary: Some(binary),
            timeout_seconds: Some(5),
            artifact_dir: None,
        });

        let result = runner
            .run(&["check".to_string(), "bad.0".to_string()], None, true)
            .await
            .unwrap();

        assert_eq!(result.status, Some(7));
        assert_eq!(result.stderr, "bad input\n");
    }

    #[tokio::test]
    async fn runner_preserves_stdout_when_json_is_invalid() {
        let binary = fake_zero("invalid-json", "printf 'not-json\\n'");
        let runner = ZeroCommandRunner::new(ZerolangConfig {
            binary: Some(binary),
            timeout_seconds: Some(5),
            artifact_dir: None,
        });

        let result = runner
            .run(&["doctor".to_string(), "--json".to_string()], None, true)
            .await
            .unwrap();

        assert_eq!(result.status, Some(0));
        assert_eq!(result.stdout, "not-json\n");
        assert_eq!(result.json, None);
    }

    #[tokio::test]
    async fn runner_reports_missing_binary() {
        let runner = ZeroCommandRunner::new(ZerolangConfig {
            binary: Some(PathBuf::from("/definitely/missing/zero")),
            timeout_seconds: Some(5),
            artifact_dir: None,
        });

        let err = runner
            .run(&["doctor".to_string()], None, false)
            .await
            .unwrap_err();

        assert!(err.to_string().contains("failed to launch Zero binary"));
    }

    #[tokio::test]
    async fn runner_reports_timeout() {
        let binary = fake_zero("timeout", "sleep 2");
        let runner = ZeroCommandRunner::new(ZerolangConfig {
            binary: Some(binary),
            timeout_seconds: Some(1),
            artifact_dir: None,
        });

        let err = runner
            .run(&["doctor".to_string()], None, false)
            .await
            .unwrap_err();

        assert!(err.to_string().contains("timed out"));
    }
}
