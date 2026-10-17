//! Explicit stable process identity, independent of a worker's in-memory map.
use super::*;
use sha2::{Digest, Sha256};

pub(super) fn durable_process_name(command_id: &str) -> anyhow::Result<String> {
    anyhow::ensure!(
        !command_id.is_empty()
            && command_id.len() <= 256
            && command_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_:-".contains(&byte)),
        "Invalid durable command identity"
    );
    let hash = Sha256::digest(command_id.as_bytes());
    let identity: String = hash
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok(format!("roder-durable-{identity}"))
}

impl BlaxelRunnerSession {
    pub(super) fn check_durable_target(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.explicit_workspace && self.fork_allocation.is_none() && !self.is_paused(),
            "Durable commands require an active original explicit-workspace session"
        );
        crate::workspace::validate_endpoint(&self.base_url, &self.endpoint())
    }

    pub(super) async fn execute_command(
        &self,
        request: RunnerCommandRequest,
        durable: bool,
    ) -> anyhow::Result<RunnerCommandResult> {
        let timeout_seconds = process_timeout_seconds(request.timeout_ms);
        let tracked = self.register_process(&request.command_id, timeout_seconds, durable)?;
        let process_name = tracked.name.clone();
        let mut process_guard = ActiveProcessGuard::new(
            self.client.clone(),
            self.endpoint(),
            request.command_id.clone(),
            tracked.clone(),
            self.running_processes.clone(),
        );
        let endpoint = match self.begin_operation().await {
            Ok(endpoint) => endpoint,
            Err(error) => {
                process_guard.complete();
                return Err(error);
            }
        };
        if self
            .running_processes
            .lock()
            .unwrap()
            .get(&request.command_id)
            .is_some_and(|process| process.cancelled)
        {
            anyhow::bail!(
                "remote command {} was cancelled before start",
                request.command_id
            );
        }
        let command = shell_join(&request.program, &request.args);
        let command = format!(
            "if [ -e {} ]; then exit 130; fi; exec {command}",
            shell_quote(&cancellation_marker(&process_name))
        );
        let environment = tagged_environment(&request, &tracked.tag);
        let cwd = request
            .cwd
            .as_ref()
            .map(|cwd| self.resolve_path(cwd))
            .unwrap_or_else(|| self.working_dir.clone());
        // If process execution errors, leave the guard armed: the remote
        // process may still be running and cancellation must settle it before
        // a post-operation standby lease is appropriate.
        let result = self
            .client
            .exec(
                &endpoint,
                &process_name,
                &command,
                Some(&cwd),
                &environment,
                timeout_seconds,
            )
            .await?;
        if result.exit_code.is_some() {
            process_guard.complete();
        }
        self.finish_operation(&endpoint).await?;
        Ok(RunnerCommandResult {
            command_id: request.command_id,
            exit_code: result.exit_code,
            stdout: result.stdout,
            stderr: result.stderr,
        })
    }
}

#[cfg(test)]
#[path = "durable_tests.rs"]
mod tests;
