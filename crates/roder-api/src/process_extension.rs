//! Process-hosted extension contract (roadmap phase 64).
//!
//! A process extension is a non-Rust child process that registers ordinary
//! extension services (inference engines, event sinks) through a manifest
//! and speaks newline-delimited JSON-RPC 2.0 over stdio. These DTOs are the
//! canonical protocol: the Rust host serializes them as-is and child
//! implementations (e.g. the Python POC) must round-trip them without raw
//! unowned JSON.
//!
//! Method names (host -> child requests unless noted):
//! - `extension/initialize`
//! - `inference/listModels`
//! - `inference/streamTurn`
//! - `inference/event` (child -> host notification)
//! - `events/handle` (host -> child notification)
//! - `extension/event` (child -> host notification)
//! - `extension/shutdown`

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::events::{EventEnvelope, ThreadId, TurnId};
use crate::extension::ProvidedService;
use crate::inference::{AgentInferenceRequest, InferenceEvent, ModelDescriptor};

/// Protocol version spoken by the host; children must echo a compatible
/// version from `extension/initialize`.
pub const PROCESS_EXTENSION_PROTOCOL_VERSION: &str = "0.1.0";

pub const METHOD_INITIALIZE: &str = "extension/initialize";
pub const METHOD_LIST_MODELS: &str = "inference/listModels";
pub const METHOD_STREAM_TURN: &str = "inference/streamTurn";
pub const METHOD_INFERENCE_EVENT: &str = "inference/event";
pub const METHOD_EVENTS_HANDLE: &str = "events/handle";
pub const METHOD_EXTENSION_EVENT: &str = "extension/event";
pub const METHOD_SHUTDOWN: &str = "extension/shutdown";

/// One `[[process_extensions]]` config entry. `env` is an explicit
/// allowlist — the host never forwards its full environment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct ProcessExtensionConfig {
    pub id: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Path to the extension manifest TOML (registry source of truth).
    pub manifest: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Milliseconds the host waits for spawn + initialize before failing.
    #[serde(default = "default_startup_timeout_ms")]
    pub startup_timeout_ms: u64,
    /// Event kinds forwarded to the child (prefix match; empty = none).
    #[serde(default)]
    pub event_filter: ProcessEventFilter,
}

fn default_enabled() -> bool {
    true
}

fn default_startup_timeout_ms() -> u64 {
    10_000
}

/// Prefix filter over canonical event kinds, e.g. `["turn.", "inference."]`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProcessEventFilter {
    #[serde(default)]
    pub kinds: Vec<String>,
}

impl ProcessEventFilter {
    pub fn matches(&self, kind: &str) -> bool {
        self.kinds.iter().any(|prefix| kind.starts_with(prefix))
    }
}

/// The manifest TOML shipped next to a process extension.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProcessExtensionManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    /// Semver requirement against [`crate::extension::SUPPORTED_EXTENSION_API_VERSION`].
    pub api_version: String,
    #[serde(default)]
    pub description: Option<String>,
    pub provides: Vec<ProcessProvidedService>,
    #[serde(default)]
    pub required_capabilities: Vec<String>,
}

/// A manifest service declaration; mirrors [`ProvidedService`] variants the
/// process host supports.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ProcessProvidedService {
    InferenceEngine { id: String },
    EventSink { id: String },
}

impl ProcessProvidedService {
    pub fn service_id(&self) -> &str {
        match self {
            ProcessProvidedService::InferenceEngine { id } => id,
            ProcessProvidedService::EventSink { id } => id,
        }
    }
}

impl From<&ProcessProvidedService> for ProvidedService {
    fn from(service: &ProcessProvidedService) -> Self {
        match service {
            ProcessProvidedService::InferenceEngine { id } => {
                ProvidedService::InferenceEngine(id.clone())
            }
            ProcessProvidedService::EventSink { id } => ProvidedService::EventSink(id.clone()),
        }
    }
}

/// `extension/initialize` params (host -> child).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessInitializeParams {
    pub protocol_version: String,
    pub api_version: String,
    pub extension_id: String,
    pub cwd: String,
    pub granted_capabilities: Vec<String>,
    /// Redacted, non-secret config the host chooses to share.
    pub config: serde_json::Value,
    pub event_filter: ProcessEventFilter,
}

/// `extension/initialize` result (child -> host).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessInitializeResult {
    pub protocol_version: String,
    /// Echo of the manifest the child believes it implements.
    pub extension_id: String,
    pub services: Vec<ProcessProvidedService>,
    /// FNV-1a checksum (hex) of the manifest TOML bytes the child shipped.
    pub manifest_checksum: String,
}

/// `inference/listModels` params.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessListModelsParams {
    pub engine_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessListModelsResult {
    pub models: Vec<ModelDescriptor>,
}

/// `inference/streamTurn` params: a canonical request plus turn provenance.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessStreamTurnParams {
    pub engine_id: String,
    pub stream_id: String,
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub request: AgentInferenceRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessStreamTurnAck {
    pub stream_id: String,
}

/// `inference/event` notification payload (child -> host). The host routes
/// by `stream_id` and converts `event` into the runtime inference stream.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessInferenceEventNotification {
    pub stream_id: String,
    pub event: InferenceEvent,
}

/// `events/handle` notification payload (host -> child).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessEventsHandleNotification {
    pub envelope: EventEnvelope,
}

/// `extension/event` notification payload (child -> host): a typed,
/// extension-owned event. Payloads must already be redacted by the child;
/// the host additionally enforces a size cap before re-emitting.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessExtensionOwnedEvent {
    pub extension_id: String,
    pub event_kind: String,
    pub schema_version: u32,
    pub payload: serde_json::Value,
}

/// Validates the child's initialize echo against the configured manifest.
/// Mismatches fail closed: the host refuses to register services from a
/// child that disagrees about identity, API, or provided services.
pub fn validate_initialize_echo(
    manifest: &ProcessExtensionManifest,
    manifest_toml: &str,
    result: &ProcessInitializeResult,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        result.protocol_version == PROCESS_EXTENSION_PROTOCOL_VERSION,
        "process extension {} speaks protocol {:?} but the host requires {:?}",
        manifest.id,
        result.protocol_version,
        PROCESS_EXTENSION_PROTOCOL_VERSION
    );
    anyhow::ensure!(
        result.extension_id == manifest.id,
        "process extension echoed id {:?} but the manifest declares {:?}",
        result.extension_id,
        manifest.id
    );
    anyhow::ensure!(
        result.services == manifest.provides,
        "process extension {} echoed services {:?} but the manifest declares {:?}",
        manifest.id,
        result.services,
        manifest.provides
    );
    let expected = manifest_checksum(manifest_toml);
    anyhow::ensure!(
        result.manifest_checksum == expected,
        "process extension {} echoed manifest checksum {:?} but the configured manifest hashes \
         to {:?}; the child is running against a different manifest",
        manifest.id,
        result.manifest_checksum,
        expected
    );
    Ok(())
}

/// Validates the manifest against the host's supported extension API.
pub fn validate_manifest(manifest: &ProcessExtensionManifest) -> anyhow::Result<()> {
    anyhow::ensure!(
        !manifest.id.trim().is_empty(),
        "process extension manifest is missing an id"
    );
    anyhow::ensure!(
        !manifest.provides.is_empty(),
        "process extension {} declares no provided services",
        manifest.id
    );
    let requirement = semver::VersionReq::parse(&manifest.api_version).map_err(|err| {
        anyhow::anyhow!(
            "process extension {} has invalid api_version {:?}: {err}",
            manifest.id,
            manifest.api_version
        )
    })?;
    let supported = semver::Version::parse(crate::extension::SUPPORTED_EXTENSION_API_VERSION)?;
    anyhow::ensure!(
        requirement.matches(&supported),
        "process extension {} requires extension API {:?} but the host supports {}",
        manifest.id,
        manifest.api_version,
        supported
    );
    Ok(())
}

/// FNV-1a 64-bit checksum (hex) of the manifest bytes. Not cryptographic —
/// it only detects manifest drift between host config and child package.
pub fn manifest_checksum(manifest_toml: &str) -> String {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    for byte in manifest_toml.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    format!("{hash:016x}")
}
