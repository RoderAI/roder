//! Contract tests for the process-extension protocol DTOs (roadmap
//! phase 64, Task 1): manifest round-trips, initialize-echo validation,
//! and canonical JSON shapes for the stdio JSON-RPC payloads.

use roder_api::inference::{
    AgentInferenceRequest, InferenceEvent, InstructionBundle, MessageDelta, ModelDescriptor,
    ModelSelection, OutputConfig, ReasoningConfig, RuntimeHints,
};
use roder_api::tools::ToolChoice;
use roder_api::process_extension::*;

fn sample_request() -> AgentInferenceRequest {
    AgentInferenceRequest {
        model: ModelSelection {
            provider: "python-chat-completions".to_string(),
            model: "gpt-5.5".to_string(),
        },
        instructions: InstructionBundle {
            system: Some("process extension protocol test".to_string()),
            developer: None,
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
