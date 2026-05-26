use roder_api::inference::{AgentInferenceRequest, ToolCallCompleted};
use roder_api::transcript::TranscriptItem;

const TASK_LEDGER_TOOL_NAME: &str = "task_ledger.update";

pub(super) fn next_tool_call(request: &AgentInferenceRequest) -> Option<ToolCallCompleted> {
    if should_start_artifact_checkpoint(request) {
        return Some(task_ledger_call("fake-tbench-artifact-ledger-open", false));
    }
    if should_write_artifact_checkpoint(request) {
        return Some(write_file_call(
            "fake-tbench-artifact-write-file",
            "primers.fasta",
            ">primer_forward\nACGTACGTACGT\n>primer_reverse\nTGCATGCATGCA\n",
        ));
    }
    if should_complete_artifact_checkpoint(request) {
        return Some(task_ledger_call(
            "fake-tbench-artifact-ledger-complete",
            true,
        ));
    }
    tbench_diagnostic_write(request)
        .map(|(path, content)| write_file_call("fake-tbench-write-file", path, content))
}

fn should_start_artifact_checkpoint(request: &AgentInferenceRequest) -> bool {
    prompt_contains(request, "FAKE_TBENCH_ARTIFACT_CHECKPOINT")
        && !has_tool_result(request, TASK_LEDGER_TOOL_NAME)
}

fn should_write_artifact_checkpoint(request: &AgentInferenceRequest) -> bool {
    prompt_contains(request, "FAKE_TBENCH_ARTIFACT_CHECKPOINT")
        && has_tool_result(request, TASK_LEDGER_TOOL_NAME)
        && !has_tool_result(request, "write_file")
}

fn should_complete_artifact_checkpoint(request: &AgentInferenceRequest) -> bool {
    prompt_contains(request, "FAKE_TBENCH_ARTIFACT_CHECKPOINT")
        && has_tool_result(request, "write_file")
        && task_ledger_update_count(request) == 1
}

fn tbench_diagnostic_write(
    request: &AgentInferenceRequest,
) -> Option<(&'static str, &'static str)> {
    if has_tool_result(request, "write_file") {
        return None;
    }
    if prompt_contains(request, "FAKE_TBENCH_GCODE_OUTPUT") {
        return Some(("out.txt", "flag{gc0d3_iz_ch4LLenGiNg}\n"));
    }
    if prompt_contains(request, "FAKE_TBENCH_SAM_JSON") {
        return Some((
            "sam-output.json",
            "{\"coords_x\":[12,34],\"coords_y\":[56,78]}\n",
        ));
    }
    if prompt_contains(request, "FAKE_TBENCH_PROTEIN_SEQUENCE") {
        return Some(("gblock.txt", "ACGTACGTACGT\n"));
    }
    if prompt_contains(request, "FAKE_TBENCH_VIDEO_FRAME") {
        return Some(("takeoff-frame.txt", "221\n"));
    }
    if prompt_contains(request, "FAKE_TBENCH_OUTPUT_DIRECTORY_HYGIENE") {
        return Some(("submission/main.rs", "fn main() {}\n"));
    }
    if prompt_contains(request, "FAKE_TBENCH_VISIBLE_VERIFIER_CONTRACT") {
        return Some(("result.txt", "GritLM/GritLM-7B\n"));
    }
    if prompt_contains(request, "FAKE_TBENCH_SERVICE_TARGET_SANITY") {
        return Some((
            "target-check.json",
            "{\"target\":\"guest\",\"guestMarker\":\"alpine-vm\",\"sshHost\":\"127.0.0.1\",\"sshPort\":2222}\n",
        ));
    }
    if prompt_contains(request, "FAKE_TBENCH_VERIFIER_DEPENDENCY_PARITY") {
        return Some((
            "verifier-parity.json",
            "{\"source\":\"visible-verifier\",\"assertions\":[\"coords_array_type\",\"row_parallel_shape\"],\"fallbackCommand\":\"python3 -m pytest tests/test_outputs.py\"}\n",
        ));
    }
    None
}

fn write_file_call(id: &str, path: &str, content: &str) -> ToolCallCompleted {
    ToolCallCompleted {
        id: id.to_string(),
        name: "write_file".to_string(),
        arguments: serde_json::json!({
            "path": path,
            "content": content
        })
        .to_string(),
    }
}

fn task_ledger_call(id: &str, complete: bool) -> ToolCallCompleted {
    let second_status = if complete { "completed" } else { "in_progress" };
    let mut write_task = serde_json::json!({
        "id": "write",
        "content": "Create primers.fasta scoreable artifact",
        "status": second_status
    });
    if complete {
        write_task["evidence"] = serde_json::json!("wrote primers.fasta");
    }
    ToolCallCompleted {
        id: id.to_string(),
        name: TASK_LEDGER_TOOL_NAME.to_string(),
        arguments: serde_json::json!({
            "tasks": [
                {
                    "id": "inspect",
                    "content": "Identify required scoreable artifact",
                    "status": "completed",
                    "evidence": "read diagnostic prompt"
                },
                write_task
            ],
            "requireCompletionEvidence": true
        })
        .to_string(),
    }
}

fn has_tool_result(request: &AgentInferenceRequest, name: &str) -> bool {
    request.transcript.iter().any(|item| {
        matches!(
            item,
            TranscriptItem::ToolResult(result) if result.name.as_deref() == Some(name)
        )
    })
}

fn task_ledger_update_count(request: &AgentInferenceRequest) -> usize {
    request
        .transcript
        .iter()
        .filter(|item| {
            matches!(
                item,
                TranscriptItem::ToolResult(result)
                    if result.name.as_deref() == Some(TASK_LEDGER_TOOL_NAME)
            )
        })
        .count()
}

fn prompt_contains(request: &AgentInferenceRequest, needle: &str) -> bool {
    request.transcript.iter().any(|item| {
        matches!(
            item,
            TranscriptItem::UserMessage(message) if message.text.contains(needle)
        )
    })
}
