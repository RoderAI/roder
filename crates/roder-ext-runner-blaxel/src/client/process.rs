use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use serde_json::{Value, json};

use super::{BlaxelClient, ProcessResponse, decode, ensure_ok};

// Most commands finish within milliseconds: check soon, then back off.
const PROCESS_POLL_FIRST: Duration = Duration::from_millis(10);
const PROCESS_POLL_MAX: Duration = Duration::from_millis(250);
const PROCESS_COMPLETION_GRACE: Duration = Duration::from_secs(30);

impl BlaxelClient {
    pub async fn exec(
        &self,
        endpoint: &str,
        process_name: &str,
        command: &str,
        working_dir: Option<&str>,
        env: &[(String, String)],
        timeout_seconds: u64,
    ) -> anyhow::Result<ProcessResponse> {
        let mut body = json!({
            "command": command,
            "name": process_name,
            "waitForCompletion": false,
            "keepAlive": true,
            "timeout": timeout_seconds.max(1),
            "restartOnFailure": false,
        });
        if let Some(dir) = working_dir {
            body["workingDir"] = json!(dir);
        }
        if !env.is_empty() {
            let map: serde_json::Map<String, Value> = env
                .iter()
                .map(|(key, value)| (key.clone(), json!(value)))
                .collect();
            body["env"] = Value::Object(map);
        }
        let mut process = self.start_process(endpoint, &body, "exec process").await?;
        let completion_deadline = Instant::now()
            .checked_add(Duration::from_secs(timeout_seconds.max(1)))
            .and_then(|deadline| deadline.checked_add(PROCESS_COMPLETION_GRACE))
            .ok_or_else(|| anyhow::anyhow!("blaxel process lease is too large"))?;
        let mut poll = PROCESS_POLL_FIRST;
        while !process.is_terminal() {
            if Instant::now() >= completion_deadline {
                bail!("blaxel process {process_name} did not terminate within its server lease");
            }
            tokio::time::sleep(poll).await;
            poll = (poll * 2).min(PROCESS_POLL_MAX);
            // Process creation and the read index are not atomic. Preserve the
            // last known running state across a transient 404; cancellation is
            // still addressable by the unique process name.
            if let Some(observed) = self.get_process(endpoint, process_name).await? {
                process = observed;
            }
        }
        Ok(process)
    }

    /// Start a bounded process whose server lease keeps a sandbox active. The
    /// caller intentionally does not poll it; its timeout bounds the grace even
    /// if the local runner exits.
    pub(crate) async fn start_keep_alive_process(
        &self,
        endpoint: &str,
        process_name: &str,
        duration_seconds: u64,
    ) -> anyhow::Result<ProcessResponse> {
        let body = json!({
            "command": format!("sleep {duration_seconds}"),
            "name": process_name,
            "waitForCompletion": false,
            "keepAlive": true,
            "timeout": duration_seconds,
            "restartOnFailure": false,
        });
        self.start_process(endpoint, &body, "start standby grace process")
            .await
    }

    async fn start_process(
        &self,
        endpoint: &str,
        body: &Value,
        action: &str,
    ) -> anyhow::Result<ProcessResponse> {
        let response = self
            .sandbox(endpoint, reqwest::Method::POST, "process")
            .json(body)
            .send()
            .await
            .with_context(|| format!("{action} on blaxel"))?;
        decode(response, action).await
    }

    pub async fn get_process(
        &self,
        endpoint: &str,
        identifier: &str,
    ) -> anyhow::Result<Option<ProcessResponse>> {
        let response = self
            .sandbox(
                endpoint,
                reqwest::Method::GET,
                &format!("process/{}", urlencoding::encode(identifier)),
            )
            .send()
            .await
            .context("get blaxel process")?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        decode(response, "get process").await.map(Some)
    }

    /// Force-kill a process by name or pid. A missing process is already no
    /// longer running, so it is reported as `false` rather than an error.
    pub async fn kill_process(&self, endpoint: &str, identifier: &str) -> anyhow::Result<bool> {
        let response = self
            .sandbox(
                endpoint,
                reqwest::Method::DELETE,
                &format!("process/{}/kill", urlencoding::encode(identifier)),
            )
            .send()
            .await
            .context("kill blaxel process")?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(false);
        }
        ensure_ok(response, "kill process").await?;
        Ok(true)
    }
}

impl BlaxelClient {
    /// Bounded metadata projection. Never return process logs or provider errors.
    pub(crate) async fn observe_process(
        &self,
        endpoint: &str,
        name: &str,
    ) -> anyhow::Result<roder_api::remote_runner::RunnerCommandObservation> {
        use roder_api::remote_runner::RunnerCommandObservation;
        #[derive(serde::Deserialize)]
        struct Metadata {
            name: String,
            status: String,
            #[serde(rename = "exitCode")]
            exit_code: Option<i32>,
        }
        let mut response = self
            .sandbox(
                endpoint,
                reqwest::Method::GET,
                &format!("process/{}", urlencoding::encode(name)),
            )
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("Durable command observation unavailable"))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(RunnerCommandObservation::NotObserved);
        }
        anyhow::ensure!(
            response.status().is_success(),
            "Durable command observation refused"
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| anyhow::anyhow!("Durable command observation unavailable"))?
        {
            anyhow::ensure!(
                bytes.len() + chunk.len() <= 65536,
                "Durable command observation exceeds budget"
            );
            bytes.extend_from_slice(&chunk);
        }
        let metadata: Metadata = serde_json::from_slice(&bytes)
            .map_err(|_| anyhow::anyhow!("Invalid durable command observation"))?;
        anyhow::ensure!(metadata.name == name, "Durable command identity changed");
        Ok(match metadata.status.as_str() {
            "running" if metadata.exit_code.is_none() => RunnerCommandObservation::Running,
            "completed" | "failed" | "killed" | "stopped" => RunnerCommandObservation::Exited {
                exit_code: metadata.exit_code,
            },
            _ => RunnerCommandObservation::Unknown,
        })
    }
}
