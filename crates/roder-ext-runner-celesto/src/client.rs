use std::time::Duration;

use anyhow::{Context, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use roder_api::remote_runner::{
    RunnerCommandRequest, RunnerCommandResult, RunnerFileReadRequest, RunnerFileReadResult,
    RunnerFileWriteRequest, RunnerPortRequest, RunnerPortResult,
};
use serde::Deserialize;

use crate::config::CelestoConfig;

/// Maximum number of 1s polls while waiting for a computer to reach `running`.
const START_TIMEOUT_SECS: u64 = 120;

#[derive(Clone, Debug, Deserialize)]
pub struct Computer {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub vcpus: Option<u32>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ExecResponse {
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub stdout: String,
    #[serde(default)]
    pub stderr: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PublishedPort {
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Clone, Debug)]
pub struct CelestoClient {
    http: reqwest::Client,
    config: CelestoConfig,
}

impl CelestoClient {
    pub fn new(http: reqwest::Client, config: CelestoConfig) -> Self {
        Self { http, config }
    }

    pub fn config(&self) -> &CelestoConfig {
        &self.config
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.config.base_url, path)
    }

    // --- Lifecycle ---------------------------------------------------------

    /// Create a fresh computer or reuse the configured one, returning it once
    /// it is running.
    pub async fn ensure_computer(&self) -> anyhow::Result<Computer> {
        let computer = match &self.config.computer_id {
            Some(id) => self.get_computer(id).await?,
            None => self.create_computer().await?,
        };
        self.ensure_running(computer).await
    }

    /// Start the computer if needed and wait until it reports `running`.
    pub async fn ensure_running(&self, computer: Computer) -> anyhow::Result<Computer> {
        match computer.status.as_str() {
            "running" => Ok(computer),
            "error" => bail!("celesto computer {} is in an error state", computer.id),
            "deleted" | "deleting" => {
                bail!("celesto computer {} has been deleted", computer.id)
            }
            "stopped" | "stopping" | "restorable" => {
                self.start_computer(&computer.id).await?;
                self.wait_until_running(&computer.id).await
            }
            _ => self.wait_until_running(&computer.id).await,
        }
    }

    pub async fn create_computer(&self) -> anyhow::Result<Computer> {
        let mut body = serde_json::Map::new();
        body.insert(
            "template_id".to_string(),
            self.config.template_id.clone().into(),
        );
        if let Some(version) = &self.config.template_version {
            body.insert("template_version".to_string(), version.clone().into());
        }
        if let Some(cpus) = self.config.cpus {
            body.insert("cpus".to_string(), cpus.into());
        }
        if let Some(memory) = self.config.memory {
            body.insert("memory".to_string(), memory.into());
        }
        if let Some(disk) = self.config.disk_size_mb {
            body.insert("disk_size_mb".to_string(), disk.into());
        }
        let response = self
            .http
            .post(self.url("/v1/computers"))
            .bearer_auth(&self.config.token)
            .json(&serde_json::Value::Object(body))
            .send()
            .await
            .context("create celesto computer")?;
        decode_json(response, "create computer").await
    }

    pub async fn get_computer(&self, id: &str) -> anyhow::Result<Computer> {
        let response = self
            .http
            .get(self.url(&format!("/v1/computers/{id}")))
            .bearer_auth(&self.config.token)
            .send()
            .await
            .context("get celesto computer")?;
        decode_json(response, "get computer").await
    }

    pub async fn start_computer(&self, id: &str) -> anyhow::Result<()> {
        let response = self
            .http
            .post(self.url(&format!("/v1/computers/{id}/start")))
            .bearer_auth(&self.config.token)
            .send()
            .await
            .context("start celesto computer")?;
        decode_empty(response, "start computer").await
    }

    pub async fn stop_computer(&self, id: &str) -> anyhow::Result<()> {
        let response = self
            .http
            .post(self.url(&format!("/v1/computers/{id}/stop")))
            .bearer_auth(&self.config.token)
            .send()
            .await
            .context("stop celesto computer")?;
        decode_empty(response, "stop computer").await
    }

    pub async fn delete_computer(&self, id: &str) -> anyhow::Result<()> {
        let response = self
            .http
            .delete(self.url(&format!("/v1/computers/{id}")))
            .bearer_auth(&self.config.token)
            .send()
            .await
            .context("delete celesto computer")?;
        decode_empty(response, "delete computer").await
    }

    async fn wait_until_running(&self, id: &str) -> anyhow::Result<Computer> {
        for _ in 0..START_TIMEOUT_SECS {
            let computer = self.get_computer(id).await?;
            match computer.status.as_str() {
                "running" => return Ok(computer),
                "error" => bail!("celesto computer {id} entered an error state while starting"),
                "deleted" | "deleting" => bail!("celesto computer {id} was deleted while starting"),
                _ => tokio::time::sleep(Duration::from_secs(1)).await,
            }
        }
        bail!("timed out waiting for celesto computer {id} to start")
    }

    // --- Execution ---------------------------------------------------------

    pub async fn exec(&self, id: &str, command: &str) -> anyhow::Result<ExecResponse> {
        let response = self
            .http
            .post(self.url(&format!("/v1/computers/{id}/exec")))
            .bearer_auth(&self.config.token)
            .json(&serde_json::json!({ "command": command }))
            .send()
            .await
            .context("exec celesto command")?;
        decode_json(response, "exec command").await
    }

    pub async fn run_command(
        &self,
        id: &str,
        request: RunnerCommandRequest,
    ) -> anyhow::Result<RunnerCommandResult> {
        let command = build_shell_command(&request, &self.config.working_dir);
        let exec = self.exec(id, &command).await?;
        Ok(RunnerCommandResult {
            command_id: request.command_id,
            exit_code: exec.exit_code,
            stdout: exec.stdout,
            stderr: exec.stderr,
        })
    }

    pub async fn ensure_working_dir(&self, id: &str) -> anyhow::Result<()> {
        let command = format!("mkdir -p -- {}", shell_quote(&self.config.working_dir));
        let exec = self.exec(id, &command).await?;
        if exec.exit_code != Some(0) {
            bail!(
                "failed to create celesto working dir {}: {}",
                self.config.working_dir,
                exec.stderr.trim()
            );
        }
        Ok(())
    }

    // --- Filesystem (via exec) --------------------------------------------

    pub async fn read_file(
        &self,
        id: &str,
        request: RunnerFileReadRequest,
    ) -> anyhow::Result<RunnerFileReadResult> {
        let path = request.path.to_string_lossy().to_string();
        let command = format!("base64 -- {}", shell_quote(&path));
        let exec = self.exec(id, &command).await?;
        if exec.exit_code != Some(0) {
            bail!("failed to read {path}: {}", exec.stderr.trim());
        }
        let cleaned: String = exec.stdout.split_whitespace().collect();
        let contents = BASE64
            .decode(cleaned.as_bytes())
            .with_context(|| format!("decode base64 contents of {path}"))?;
        Ok(RunnerFileReadResult {
            path: request.path,
            contents,
        })
    }

    pub async fn write_file(&self, id: &str, request: RunnerFileWriteRequest) -> anyhow::Result<()> {
        let path = request.path.to_string_lossy().to_string();
        let encoded = BASE64.encode(&request.contents);
        let quoted = shell_quote(&path);
        let command = format!(
            "mkdir -p -- \"$(dirname -- {quoted})\" && printf %s {} | base64 -d > {quoted}",
            shell_quote(&encoded)
        );
        let exec = self.exec(id, &command).await?;
        if exec.exit_code != Some(0) {
            bail!("failed to write {path}: {}", exec.stderr.trim());
        }
        Ok(())
    }

    // --- Ports -------------------------------------------------------------

    pub async fn expose_port(
        &self,
        id: &str,
        request: RunnerPortRequest,
    ) -> anyhow::Result<RunnerPortResult> {
        let published = self.publish_port(id, request.port).await?;
        Ok(RunnerPortResult {
            port: published.port.unwrap_or(request.port),
            url: published.url,
        })
    }

    pub async fn publish_port(&self, id: &str, port: u16) -> anyhow::Result<PublishedPort> {
        let response = self
            .http
            .post(self.url(&format!("/v1/computers/{id}/ports")))
            .bearer_auth(&self.config.token)
            .json(&serde_json::json!({ "port": port }))
            .send()
            .await
            .context("publish celesto port")?;
        decode_json(response, "publish port").await
    }

    pub async fn unpublish_port(&self, id: &str, port: u16) -> anyhow::Result<()> {
        let response = self
            .http
            .delete(self.url(&format!("/v1/computers/{id}/ports/{port}")))
            .bearer_auth(&self.config.token)
            .send()
            .await
            .context("unpublish celesto port")?;
        decode_empty(response, "unpublish port").await
    }
}

/// Build a single shell command for `computers.exec`, honoring cwd and env.
fn build_shell_command(request: &RunnerCommandRequest, working_dir: &str) -> String {
    let cwd = request
        .cwd
        .as_ref()
        .map(|path| path.to_string_lossy().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| working_dir.to_string());
    let mut parts = vec![format!("cd {} &&", shell_quote(&cwd))];
    for (key, value) in &request.env {
        parts.push(format!("{key}={}", shell_quote(value)));
    }
    parts.push(shell_quote(&request.program));
    for arg in &request.args {
        parts.push(shell_quote(arg));
    }
    parts.join(" ")
}

/// Minimal POSIX single-quote escaping for values interpolated into a command.
fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    let safe = value.chars().all(|c| {
        c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '/' | '.' | ':' | '@' | '+' | ',')
    });
    if safe {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

async fn decode_json<T: for<'de> Deserialize<'de>>(
    response: reqwest::Response,
    operation: &str,
) -> anyhow::Result<T> {
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .with_context(|| format!("read celesto {operation} response"))?;
    if !status.is_success() {
        bail!(
            "celesto {operation} failed with status {status}: {}",
            String::from_utf8_lossy(&bytes)
        );
    }
    serde_json::from_slice(&bytes).with_context(|| format!("decode celesto {operation} response"))
}

async fn decode_empty(response: reqwest::Response, operation: &str) -> anyhow::Result<()> {
    let status = response.status();
    let bytes = response.bytes().await.unwrap_or_default();
    if !status.is_success() {
        bail!(
            "celesto {operation} failed with status {status}: {}",
            String::from_utf8_lossy(&bytes)
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_quote_handles_special_characters() {
        assert_eq!(shell_quote("plain.txt"), "plain.txt");
        assert_eq!(shell_quote(""), "''");
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
    }

    #[test]
    fn build_shell_command_includes_cwd_env_and_args() {
        let request = RunnerCommandRequest {
            command_id: "cmd-1".to_string(),
            program: "python3".to_string(),
            args: vec!["-c".to_string(), "print(2+2)".to_string()],
            cwd: None,
            env: vec![("RUST_LOG".to_string(), "info".to_string())],
        };
        let command = build_shell_command(&request, "/root/roder");
        assert_eq!(command, "cd /root/roder && RUST_LOG=info python3 -c 'print(2+2)'");
    }
}
