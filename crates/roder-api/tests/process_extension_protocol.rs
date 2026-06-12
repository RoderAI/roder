//! Contract tests for the process-extension protocol DTOs (roadmap
//! phase 64, Task 1): manifest round-trips, initialize-echo validation,
//! and canonical JSON shapes for the stdio JSON-RPC payloads.

use roder_api::inference::{
    AgentInferenceRequest, InferenceEvent, InstructionBundle, MessageDelta, ModelDescriptor,
    ModelSelection, OutputConfig, ReasoningConfig, RuntimeHints,
};
use roder_api::process_extension::*;
use roder_api::tools::ToolChoice;

fn sample_request() -> AgentInferenceRequest {
    AgentInferenceRequest {
        model: ModelSelection {
            provider: "python-chat-completions".to_string(),
            model: "gpt-5.5".to_string(),
        },
        instructions: InstructionBundle {
            system: Some("process extension protocol test".to_string()),
            developer: None,
            developer_context: None,
        },
        transcript: vec![roder_api::transcript::TranscriptItem::UserMessage(
            roder_api::transcript::UserMessage::text("hello"),
        )],
        tools: Vec::new(),
        tool_choice: ToolChoice::Auto,
        reasoning: ReasoningConfig {
            enabled: false,
            level: None,
        },
        output: OutputConfig {
            max_tokens: Some(64),
            temperature: None,
            top_p: None,
            response_format: None,
        },
        runtime: RuntimeHints::default(),
        metadata: serde_json::json!({}),
    }
}

const MANIFEST_TOML: &str = r#"
id = "roder-ext-python-chat-completions"
name = "Python Chat Completions"
version = "0.1.0"
api_version = "^0.1"
description = "Process-hosted Python OpenAI-compatible chat-completions provider"

provides = [
  { type = "inference_engine", id = "python-chat-completions" },
  { type = "event_sink", id = "python-chat-completions-events" },
]

required_capabilities = [
  "network.api.openai.com",
  "secret.read.PY_CHAT_COMPLETIONS_API_KEY",
  "events.read.turn",
  "events.emit.extension",
]
"#;

fn manifest() -> ProcessExtensionManifest {
    toml::from_str(MANIFEST_TOML).unwrap()
}

fn initialize_result() -> ProcessInitializeResult {
    ProcessInitializeResult {
        protocol_version: PROCESS_EXTENSION_PROTOCOL_VERSION.to_string(),
        extension_id: "roder-ext-python-chat-completions".to_string(),
        services: manifest().provides,
        manifest_checksum: manifest_checksum(MANIFEST_TOML),
    }
}

#[test]
fn manifest_toml_round_trips_to_provided_services() {
    let manifest = manifest();
    assert_eq!(manifest.id, "roder-ext-python-chat-completions");
    assert_eq!(manifest.provides.len(), 2);

    let services: Vec<roder_api::extension::ProvidedService> =
        manifest.provides.iter().map(Into::into).collect();
    assert_eq!(
        services,
        vec![
            roder_api::extension::ProvidedService::InferenceEngine(
                "python-chat-completions".to_string()
            ),
            roder_api::extension::ProvidedService::EventSink(
                "python-chat-completions-events".to_string()
            ),
        ]
    );

    validate_manifest(&manifest).unwrap();
}

#[test]
fn manifest_validation_rejects_incompatible_api_and_empty_services() {
    let mut incompatible = manifest();
    incompatible.api_version = "^9.9".to_string();
    let error = validate_manifest(&incompatible).unwrap_err().to_string();
    assert!(error.contains("requires extension API"), "{error}");

    let mut empty = manifest();
    empty.provides.clear();
    let error = validate_manifest(&empty).unwrap_err().to_string();
    assert!(error.contains("no provided services"), "{error}");
}

#[test]
fn initialize_echo_validation_fails_closed_on_mismatches() {
    let manifest = manifest();
    validate_initialize_echo(&manifest, MANIFEST_TOML, &initialize_result()).unwrap();

    let mut wrong_protocol = initialize_result();
    wrong_protocol.protocol_version = "0.0.1".to_string();
    let error = validate_initialize_echo(&manifest, MANIFEST_TOML, &wrong_protocol)
        .unwrap_err()
        .to_string();
    assert!(error.contains("speaks protocol"), "{error}");

    let mut wrong_id = initialize_result();
    wrong_id.extension_id = "someone-else".to_string();
    let error = validate_initialize_echo(&manifest, MANIFEST_TOML, &wrong_id)
        .unwrap_err()
        .to_string();
    assert!(error.contains("echoed id"), "{error}");

    let mut wrong_services = initialize_result();
    wrong_services.services.pop();
    let error = validate_initialize_echo(&manifest, MANIFEST_TOML, &wrong_services)
        .unwrap_err()
        .to_string();
    assert!(error.contains("echoed services"), "{error}");

    let mut wrong_checksum = initialize_result();
    wrong_checksum.manifest_checksum = "deadbeefdeadbeef".to_string();
    let error = validate_initialize_echo(&manifest, MANIFEST_TOML, &wrong_checksum)
        .unwrap_err()
        .to_string();
    assert!(error.contains("different manifest"), "{error}");
}

#[test]
fn config_defaults_and_event_filter_behave() {
    let config: ProcessExtensionConfig = toml::from_str(
        r#"
id = "python-chat-completions"
manifest = "examples/non-rust-extensions/python-chat-completions/roder-extension.toml"
command = "python3"
args = ["-m", "roder_python_chat_provider"]
event_filter = { kinds = ["turn.", "inference."] }
"#,
    )
    .unwrap();
    assert!(config.enabled);
    assert_eq!(config.startup_timeout_ms, 10_000);
    assert!(config.env.is_empty(), "no implicit env forwarding");
    assert!(config.event_filter.matches("turn.started"));
    assert!(config.event_filter.matches("inference.started"));
    assert!(!config.event_filter.matches("tool.call_started"));
    assert!(!ProcessEventFilter::default().matches("turn.started"));
}

#[test]
fn protocol_payloads_use_canonical_json_names() {
    let params = ProcessStreamTurnParams {
        engine_id: "python-chat-completions".to_string(),
        stream_id: "stream-1".to_string(),
        thread_id: "thread-1".to_string(),
        turn_id: "turn-1".to_string(),
        request: sample_request(),
    };
    let json = serde_json::to_value(&params).unwrap();
    assert_eq!(json["engineId"], "python-chat-completions");
    assert_eq!(json["streamId"], "stream-1");
    assert_eq!(json["threadId"], "thread-1");
    assert_eq!(json["turnId"], "turn-1");
    assert!(json.get("request").is_some());

    let notification = ProcessInferenceEventNotification {
        stream_id: "stream-1".to_string(),
        event: InferenceEvent::MessageDelta(MessageDelta {
            text: "hello".to_string(),
            phase: None,
        }),
    };
    let round_trip: ProcessInferenceEventNotification =
        serde_json::from_value(serde_json::to_value(&notification).unwrap()).unwrap();
    assert_eq!(round_trip, notification);

    let models = ProcessListModelsResult {
        models: vec![ModelDescriptor {
            id: "gpt-5.5".to_string(),
            name: "GPT 5.5".to_string(),
            context_window: Some(200_000),
            default_reasoning: None,
            supported_reasoning: Vec::new(),
        }],
    };
    let json = serde_json::to_value(&models).unwrap();
    assert_eq!(json["models"][0]["id"], "gpt-5.5");
}

#[test]
fn extension_owned_events_round_trip_with_schema_version() {
    let event = ProcessExtensionOwnedEvent {
        extension_id: "roder-ext-python-chat-completions".to_string(),
        event_kind: "provider.turn_observed".to_string(),
        schema_version: 1,
        payload: serde_json::json!({ "turns": 1 }),
    };
    let round_trip: ProcessExtensionOwnedEvent =
        serde_json::from_value(serde_json::to_value(&event).unwrap()).unwrap();
    assert_eq!(round_trip, event);
}

const DISPATCHER_MANIFEST_TOML: &str = r#"
id = "roder-ext-cursor-sdk"
name = "Cursor SDK Agents"
version = "0.1.0"
api_version = "^0.1"
description = "Process-hosted TypeScript extension wrapping @cursor/sdk"

provides = [
  { type = "subagent_dispatcher", id = "cursor-cloud" },
  { type = "task_executor", id = "cursor-cloud-agent" },
  { type = "event_sink", id = "cursor-sdk-events" },
]

required_capabilities = [
  "network.api.cursor.com",
  "secret.read.CURSOR_API_KEY",
  "events.read.turn",
  "events.emit.extension",
]
"#;

#[test]
fn dispatcher_and_task_manifest_round_trips_and_echo_fails_closed() {
    let manifest: ProcessExtensionManifest = toml::from_str(DISPATCHER_MANIFEST_TOML).unwrap();
    validate_manifest(&manifest).unwrap();

    let services: Vec<roder_api::extension::ProvidedService> =
        manifest.provides.iter().map(Into::into).collect();
    assert_eq!(
        services,
        vec![
            roder_api::extension::ProvidedService::SubagentDispatcher("cursor-cloud".to_string()),
            roder_api::extension::ProvidedService::TaskExecutor("cursor-cloud-agent".to_string()),
            roder_api::extension::ProvidedService::EventSink("cursor-sdk-events".to_string()),
        ]
    );
    assert_eq!(manifest.provides[0].service_id(), "cursor-cloud");

    let good = ProcessInitializeResult {
        protocol_version: PROCESS_EXTENSION_PROTOCOL_VERSION.to_string(),
        extension_id: manifest.id.clone(),
        services: manifest.provides.clone(),
        manifest_checksum: manifest_checksum(DISPATCHER_MANIFEST_TOML),
    };
    validate_initialize_echo(&manifest, DISPATCHER_MANIFEST_TOML, &good).unwrap();

    // A child that drops the task executor from its echo is refused.
    let mut missing_service = good.clone();
    missing_service
        .services
        .retain(|service| service.service_id() != "cursor-cloud-agent");
    let error = validate_initialize_echo(&manifest, DISPATCHER_MANIFEST_TOML, &missing_service)
        .unwrap_err()
        .to_string();
    assert!(error.contains("echoed services"), "{error}");

    // The protocol version is the bumped canonical one, with no aliases.
    assert_eq!(PROCESS_EXTENSION_PROTOCOL_VERSION, "0.2.0");
    let mut stale_protocol = good;
    stale_protocol.protocol_version = "0.1.0".to_string();
    let error = validate_initialize_echo(&manifest, DISPATCHER_MANIFEST_TOML, &stale_protocol)
        .unwrap_err()
        .to_string();
    assert!(error.contains("speaks protocol"), "{error}");
}

#[test]
fn subagent_dispatch_payloads_use_canonical_json_names() {
    let params = ProcessSubagentDispatchParams {
        dispatcher_id: "cursor-cloud".to_string(),
        dispatch_id: "dispatch-1".to_string(),
        parent_thread_id: "thread-1".to_string(),
        parent_turn_id: "turn-1".to_string(),
        request: roder_api::subagents::SubagentRequest {
            description: "Fix the bug remotely".to_string(),
            prompt: "Fix the flaky test".to_string(),
            subagent_type: Some("cursor-cloud".to_string()),
            model: Some("composer-2.5".to_string()),
            tools: None,
            lane: None,
            max_concurrent: None,
            allowed_tools: None,
            parent_deadline_seconds: None,
            inputs: Some(serde_json::json!({
                "repoUrl": "https://github.com/example-org/example-repo",
            })),
            timeout_seconds: Some(600),
        },
    };
    let json = serde_json::to_value(&params).unwrap();
    assert_eq!(json["dispatcherId"], "cursor-cloud");
    assert_eq!(json["dispatchId"], "dispatch-1");
    assert_eq!(json["parentThreadId"], "thread-1");
    assert_eq!(json["parentTurnId"], "turn-1");
    assert_eq!(json["request"]["prompt"], "Fix the flaky test");
    let round_trip: ProcessSubagentDispatchParams = serde_json::from_value(json).unwrap();
    assert_eq!(round_trip, params);

    let completed = ProcessSubagentEventNotification {
        dispatch_id: "dispatch-1".to_string(),
        event: ProcessSubagentEvent::Completed {
            result: Box::new(roder_api::subagents::SubagentResult {
                thread_id: "bc-agent-1".to_string(),
                turn_id: "request-1".to_string(),
                agent_type: "cursor-cloud".to_string(),
                model: Some("composer-2.5".to_string()),
                final_message: "All done".to_string(),
                usage: None,
                exit_reason: roder_api::subagents::SubagentExitReason::Completed,
                transcript: None,
                metadata: serde_json::json!({ "agentId": "bc-agent-1" }),
            }),
        },
    };
    let json = serde_json::to_value(&completed).unwrap();
    assert_eq!(json["dispatchId"], "dispatch-1");
    assert_eq!(json["event"]["type"], "completed");
    let round_trip: ProcessSubagentEventNotification = serde_json::from_value(json).unwrap();
    assert_eq!(round_trip, completed);

    let status = ProcessSubagentEvent::Status {
        status: "RUNNING".to_string(),
        detail: Some("cloud VM provisioning".to_string()),
    };
    let json = serde_json::to_value(&status).unwrap();
    assert_eq!(json["type"], "status");
    assert_eq!(json["status"], "RUNNING");

    let cancel = ProcessSubagentCancelParams {
        dispatcher_id: "cursor-cloud".to_string(),
        dispatch_id: "dispatch-1".to_string(),
        reason: Some("parent turn cancelled".to_string()),
    };
    let json = serde_json::to_value(&cancel).unwrap();
    assert_eq!(json["dispatcherId"], "cursor-cloud");
    assert_eq!(json["reason"], "parent turn cancelled");
}

#[test]
fn task_execute_payloads_use_canonical_json_names() {
    let spec_result = ProcessTaskSpecResult {
        spec: roder_api::tasks::TaskSpec {
            kind: "cursor-cloud-agent".to_string(),
            description: "Remote Cursor cloud agent".to_string(),
            input_schema: serde_json::json!({ "type": "object" }),
            default_timeout_seconds: Some(1800),
            metadata: serde_json::json!({}),
        },
    };
    let json = serde_json::to_value(&spec_result).unwrap();
    assert_eq!(json["spec"]["kind"], "cursor-cloud-agent");

    let params = ProcessTaskExecuteParams {
        executor_id: "cursor-cloud-agent".to_string(),
        execution_id: "execution-1".to_string(),
        task_id: "task-1".to_string(),
        thread_id: Some("thread-1".to_string()),
        turn_id: None,
        workspace_root: Some("/workspace".to_string()),
        input: serde_json::json!({ "prompt": "Summarize the repo" }),
    };
    let json = serde_json::to_value(&params).unwrap();
    assert_eq!(json["executorId"], "cursor-cloud-agent");
    assert_eq!(json["executionId"], "execution-1");
    assert_eq!(json["taskId"], "task-1");
    assert_eq!(json["threadId"], "thread-1");
    assert!(json.get("turnId").is_none(), "absent options are omitted");
    let round_trip: ProcessTaskExecuteParams = serde_json::from_value(json).unwrap();
    assert_eq!(round_trip, params);

    let output = ProcessTaskEventNotification {
        execution_id: "execution-1".to_string(),
        event: ProcessTaskEvent::Output {
            stream: roder_api::tasks::TaskOutputStream::Log,
            chunk: "status: RUNNING".to_string(),
        },
    };
    let json = serde_json::to_value(&output).unwrap();
    assert_eq!(json["executionId"], "execution-1");
    assert_eq!(json["event"]["type"], "output");
    assert_eq!(json["event"]["stream"], "log");

    let completed = ProcessTaskEventNotification {
        execution_id: "execution-1".to_string(),
        event: ProcessTaskEvent::Completed {
            result: roder_api::tasks::TaskExecutionResult::success(
                serde_json::json!({ "agentId": "bc-agent-1" }),
            ),
        },
    };
    let round_trip: ProcessTaskEventNotification =
        serde_json::from_value(serde_json::to_value(&completed).unwrap()).unwrap();
    assert_eq!(round_trip, completed);

    let failed = ProcessTaskEvent::Failed {
        error: "cloud agent errored".to_string(),
    };
    let json = serde_json::to_value(&failed).unwrap();
    assert_eq!(json["type"], "failed");
}
