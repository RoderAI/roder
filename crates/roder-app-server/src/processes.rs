use roder_protocol::{
    JsonRpcError, ProcessesGetParams, ProcessesGetResult, ProcessesListParams, ProcessesListResult,
    ProcessesStopAllParams, ProcessesStopAllResult, ProcessesStopParams, ProcessesStopResult,
    ProcessesSubscribeResult,
};

use crate::AppServer;

impl AppServer {
    pub(crate) async fn handle_processes_list(
        &self,
        params: ProcessesListParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        Ok(serde_json::to_value(ProcessesListResult {
            processes: self.tasks.processes().list(params.include_completed).await,
        })
        .unwrap())
    }

    pub(crate) async fn handle_processes_get(
        &self,
        params: ProcessesGetParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let registry = self.tasks.processes();
        let output = limit_output_tail(
            registry.output(&params.process_id).await,
            params.output_bytes,
        );
        Ok(serde_json::to_value(ProcessesGetResult {
            process: registry.get(&params.process_id).await,
            output,
        })
        .unwrap())
    }

    pub(crate) async fn handle_processes_stop(
        &self,
        params: ProcessesStopParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let result = self
            .tasks
            .processes()
            .stop(&params.process_id, params.reason)
            .await
            .map_err(internal_error)?;
        Ok(serde_json::to_value(ProcessesStopResult { result }).unwrap())
    }

    pub(crate) async fn handle_processes_stop_all(
        &self,
        params: ProcessesStopAllParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        Ok(serde_json::to_value(ProcessesStopAllResult {
            results: self.tasks.processes().stop_all(params.reason).await,
        })
        .unwrap())
    }

    pub(crate) async fn handle_processes_subscribe(
        &self,
    ) -> Result<serde_json::Value, JsonRpcError> {
        Ok(serde_json::to_value(ProcessesSubscribeResult {
            subscribed: true,
            event_kinds: vec![
                "process.started".to_string(),
                "process.output".to_string(),
                "process.exited".to_string(),
                "process.stopping".to_string(),
                "process.stopped".to_string(),
                "process.failed".to_string(),
            ],
        })
        .unwrap())
    }
}

fn limit_output_tail(
    mut output: Vec<roder_api::processes::ProcessOutput>,
    output_bytes: Option<usize>,
) -> Vec<roder_api::processes::ProcessOutput> {
    let Some(limit) = output_bytes else {
        return output;
    };
    let mut total = 0usize;
    let mut keep_from = output.len();
    for (idx, item) in output.iter().enumerate().rev() {
        total = total.saturating_add(item.chunk.len());
        keep_from = idx;
        if total >= limit {
            break;
        }
    }
    output.drain(0..keep_from);
    output
}

fn internal_error(err: impl std::fmt::Display) -> JsonRpcError {
    let details = format!("{err:#}");
    JsonRpcError {
        code: -32000,
        message: details.clone(),
        data: Some(serde_json::json!({ "details": details })),
    }
}

#[cfg(test)]
mod tests {
    use roder_api::processes::ProcessOutput;
    use roder_api::tasks::TaskOutputStream;

    use super::*;

    #[test]
    fn processes_output_tail_respects_byte_limit() {
        let output = vec![
            output("process-1", "alpha"),
            output("process-1", "beta"),
            output("process-1", "gamma"),
        ];

        let limited = limit_output_tail(output, Some(9));

        assert_eq!(limited.len(), 2);
        assert_eq!(limited[0].chunk, "beta");
        assert_eq!(limited[1].chunk, "gamma");
    }

    fn output(process_id: &str, chunk: &str) -> ProcessOutput {
        ProcessOutput {
            process_id: process_id.to_string(),
            stream: TaskOutputStream::Stdout,
            chunk: chunk.to_string(),
            dropped_bytes: 0,
            thread_id: None,
            turn_id: None,
            timestamp: time::OffsetDateTime::UNIX_EPOCH,
        }
    }
}
