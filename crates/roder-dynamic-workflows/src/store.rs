use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use crate::host_api::WorkflowCheckpoint;
use crate::model::{WorkflowRuntimeError, WorkflowRuntimeErrorKind, WorkflowRuntimeResult};
use roder_api::subagents::SubagentResult;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone)]
pub struct WorkflowCheckpointStore {
    root: PathBuf,
}

impl WorkflowCheckpointStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn append_checkpoint(
        &self,
        run_id: &str,
        checkpoint: &WorkflowCheckpoint,
    ) -> WorkflowRuntimeResult<()> {
        let dir = self.run_dir(run_id);
        fs::create_dir_all(&dir).map_err(store_error)?;
        let path = dir.join("checkpoints.jsonl");
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(store_error)?;
        let line = serde_json::to_string(checkpoint).map_err(|err| {
            WorkflowRuntimeError::new(
                WorkflowRuntimeErrorKind::Store,
                format!("checkpoint is not serializable: {err}"),
            )
        })?;
        writeln!(file, "{line}").map_err(store_error)?;
        Ok(())
    }

    pub fn read_checkpoints(&self, run_id: &str) -> WorkflowRuntimeResult<Vec<WorkflowCheckpoint>> {
        let path = self.run_dir(run_id).join("checkpoints.jsonl");
        if !path.exists() {
            return Ok(Vec::new());
        }

        let file = fs::File::open(path).map_err(store_error)?;
        let reader = BufReader::new(file);
        let mut checkpoints = Vec::new();
        for line in reader.lines() {
            let line = line.map_err(store_error)?;
            if line.trim().is_empty() {
                continue;
            }
            let checkpoint = serde_json::from_str(&line).map_err(|err| {
                WorkflowRuntimeError::new(
                    WorkflowRuntimeErrorKind::Store,
                    format!("invalid checkpoint record: {err}"),
                )
            })?;
            checkpoints.push(checkpoint);
        }
        Ok(checkpoints)
    }

    pub fn append_agent_result(
        &self,
        run_id: &str,
        result: &WorkflowCachedAgentResult,
    ) -> WorkflowRuntimeResult<()> {
        let dir = self.run_dir(run_id);
        fs::create_dir_all(&dir).map_err(store_error)?;
        let path = dir.join("agent-results.jsonl");
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(store_error)?;
        let line = serde_json::to_string(result).map_err(|err| {
            WorkflowRuntimeError::new(
                WorkflowRuntimeErrorKind::Store,
                format!("agent result is not serializable: {err}"),
            )
        })?;
        writeln!(file, "{line}").map_err(store_error)?;
        Ok(())
    }

    pub fn read_agent_results(
        &self,
        run_id: &str,
    ) -> WorkflowRuntimeResult<Vec<WorkflowCachedAgentResult>> {
        let path = self.run_dir(run_id).join("agent-results.jsonl");
        if !path.exists() {
            return Ok(Vec::new());
        }

        let file = fs::File::open(path).map_err(store_error)?;
        let reader = BufReader::new(file);
        let mut results = Vec::new();
        for line in reader.lines() {
            let line = line.map_err(store_error)?;
            if line.trim().is_empty() {
                continue;
            }
            let result = serde_json::from_str(&line).map_err(|err| {
                WorkflowRuntimeError::new(
                    WorkflowRuntimeErrorKind::Store,
                    format!("invalid agent result record: {err}"),
                )
            })?;
            results.push(result);
        }
        Ok(results)
    }

    pub fn find_agent_result(
        &self,
        run_id: &str,
        key: &WorkflowAgentCacheKey,
    ) -> WorkflowRuntimeResult<Option<WorkflowCachedAgentResult>> {
        Ok(self
            .read_agent_results(run_id)?
            .into_iter()
            .rev()
            .find(|record| &record.key == key))
    }

    pub fn invalidate_agent_results(
        &self,
        run_id: &str,
        agent_id: &str,
    ) -> WorkflowRuntimeResult<usize> {
        let path = self.run_dir(run_id).join("agent-results.jsonl");
        if !path.exists() {
            return Ok(0);
        }

        let results = self.read_agent_results(run_id)?;
        let original_len = results.len();
        let retained = results
            .into_iter()
            .filter(|record| record.key.agent_id != agent_id)
            .collect::<Vec<_>>();
        let removed = original_len.saturating_sub(retained.len());
        let mut file = OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(path)
            .map_err(store_error)?;
        for result in retained {
            let line = serde_json::to_string(&result).map_err(|err| {
                WorkflowRuntimeError::new(
                    WorkflowRuntimeErrorKind::Store,
                    format!("agent result is not serializable: {err}"),
                )
            })?;
            writeln!(file, "{line}").map_err(store_error)?;
        }
        Ok(removed)
    }

    fn run_dir(&self, run_id: &str) -> PathBuf {
        self.root
            .join("dynamic-workflows")
            .join("runs")
            .join(run_id)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowAgentCacheKey {
    pub run_id: String,
    pub phase_id: String,
    pub agent_id: String,
    pub prompt_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default)]
    pub tool_scope: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowCachedAgentResult {
    pub key: WorkflowAgentCacheKey,
    pub result: SubagentResult,
    #[serde(with = "time::serde::rfc3339")]
    pub completed_at: OffsetDateTime,
}

fn store_error(error: std::io::Error) -> WorkflowRuntimeError {
    WorkflowRuntimeError::new(WorkflowRuntimeErrorKind::Store, error.to_string())
}
