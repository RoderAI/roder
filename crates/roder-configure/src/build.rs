use std::path::{Path, PathBuf};

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildOptions {
    pub offline: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildSummary {
    pub binary_path: PathBuf,
    pub install_hint: String,
    pub extension_summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CargoFailure {
    pub status: Option<i32>,
    pub first_error_block: String,
}

pub async fn build_release(
    crate_dir: impl AsRef<Path>,
    package_name: &str,
    extensions: &[String],
    options: BuildOptions,
) -> Result<BuildSummary, CargoFailure> {
    let crate_dir = crate_dir.as_ref();
    let mut command = cargo_build_command(crate_dir, options.offline);
    let mut child = command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|err| CargoFailure {
            status: None,
            first_error_block: err.to_string(),
        })?;

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let stdout_task = tokio::spawn(read_lines(stdout));
    let stderr_task = tokio::spawn(read_lines(stderr));
    let status = child.wait().await.map_err(|err| CargoFailure {
        status: None,
        first_error_block: err.to_string(),
    })?;
    let mut output = stdout_task.await.unwrap_or_default();
    output.push_str(&stderr_task.await.unwrap_or_default());

    if !status.success() {
        return Err(CargoFailure {
            status: status.code(),
            first_error_block: first_error_block(&output),
        });
    }

    let binary_path = crate_dir.join("target/release").join(package_name);
    Ok(BuildSummary {
        binary_path: binary_path.clone(),
        install_hint: format!("mv {} ~/.local/bin/{package_name}", binary_path.display()),
        extension_summary: format!("built with {}", extensions.join(", ")),
    })
}

pub fn cargo_build_args(offline: bool) -> Vec<String> {
    let mut args = vec!["build".to_string(), "--release".to_string()];
    if offline {
        args.push("--offline".to_string());
    }
    args
}

pub fn offline_from_env() -> bool {
    std::env::var("RODER_CONFIGURE_OFFLINE").is_ok_and(|value| value == "1")
}

pub fn first_error_block(output: &str) -> String {
    let mut block = Vec::new();
    let mut capturing = false;
    for line in output.lines() {
        if line.starts_with("error") || line.contains("error:") {
            capturing = true;
        }
        if capturing {
            if line.trim().is_empty() && !block.is_empty() {
                break;
            }
            block.push(line.to_string());
        }
    }
    if block.is_empty() {
        output.lines().take(8).collect::<Vec<_>>().join("\n")
    } else {
        block.join("\n")
    }
}

fn cargo_build_command(crate_dir: &Path, offline: bool) -> Command {
    let mut command = Command::new("cargo");
    command
        .args(cargo_build_args(offline))
        .current_dir(crate_dir);
    command
}

async fn read_lines(stream: impl tokio::io::AsyncRead + Unpin) -> String {
    let mut reader = BufReader::new(stream).lines();
    let mut out = String::new();
    while let Ok(Some(line)) = reader.next_line().await {
        out.push_str(&line);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_args_honor_offline_flag() {
        assert_eq!(cargo_build_args(false), ["build", "--release"]);
        assert_eq!(cargo_build_args(true), ["build", "--release", "--offline"]);
    }

    #[test]
    fn build_failure_extracts_first_error_block() {
        let output = "checking\nerror[E0000]: bad thing\n  --> src/main.rs:1\n\nwarning later";

        assert_eq!(
            first_error_block(output),
            "error[E0000]: bad thing\n  --> src/main.rs:1"
        );
    }

    #[tokio::test]
    async fn build_live_openai_only_distribution_when_enabled() {
        if std::env::var("RODER_CONFIGURE_LIVE_BUILD").ok().as_deref() != Some("1") {
            return;
        }
        let crate_dir = std::env::current_dir().unwrap();
        let result = build_release(
            crate_dir,
            "roder-configure",
            &["fixture".to_string()],
            BuildOptions { offline: false },
        )
        .await;

        assert!(result.is_ok());
    }
}
