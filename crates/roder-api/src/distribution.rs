use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DistributionEntry {
    pub id: String,
    pub crate_name: String,
    pub category: ExtensionCategory,
    pub display_name: String,
    pub description: String,
    #[serde(default)]
    pub default_in_profiles: Vec<String>,
    #[serde(default)]
    pub required_env: Vec<String>,
    #[serde(default)]
    pub optional_env: Vec<String>,
    #[serde(default)]
    pub conflicts_with: Vec<String>,
    #[serde(default)]
    pub required_capabilities: Vec<String>,
    pub extension_path: String,
    #[serde(default)]
    pub docs_url: Option<String>,
    #[serde(default)]
    pub extras: serde_json::Value,
}

/**
 * Extension category in distribution metadata. Serializes as a plain
 * kebab-case string; unknown category names deserialize into
 * [`ExtensionCategory::Other`] so new crates can declare novel categories
 * (e.g. `inference-router`) without breaking older tooling.
 */
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ExtensionCategory {
    InferenceEngine,
    WireDialect,
    ThreadStore,
    CheckpointStore,
    MemoryStore,
    EmbeddingProvider,
    ContextProvider,
    ContextPlanner,
    ToolProvider,
    PolicyContributor,
    SandboxBackend,
    EventSink,
    TaskExecutor,
    StatusSegment,
    PaletteSource,
    SpeechTranscriber,
    SpeechSynthesizer,
    MediaGenerator,
    Other(String),
}

impl ExtensionCategory {
    pub fn as_str(&self) -> &str {
        match self {
            Self::InferenceEngine => "inference-engine",
            Self::WireDialect => "wire-dialect",
            Self::ThreadStore => "thread-store",
            Self::CheckpointStore => "checkpoint-store",
            Self::MemoryStore => "memory-store",
            Self::EmbeddingProvider => "embedding-provider",
            Self::ContextProvider => "context-provider",
            Self::ContextPlanner => "context-planner",
            Self::ToolProvider => "tool-provider",
            Self::PolicyContributor => "policy-contributor",
            Self::SandboxBackend => "sandbox-backend",
            Self::EventSink => "event-sink",
            Self::TaskExecutor => "task-executor",
            Self::StatusSegment => "status-segment",
            Self::PaletteSource => "palette-source",
            Self::SpeechTranscriber => "speech-transcriber",
            Self::SpeechSynthesizer => "speech-synthesizer",
            Self::MediaGenerator => "media-generator",
            Self::Other(name) => name,
        }
    }

    fn from_name(name: &str) -> Self {
        match name {
            "inference-engine" => Self::InferenceEngine,
            "wire-dialect" => Self::WireDialect,
            "thread-store" => Self::ThreadStore,
            "checkpoint-store" => Self::CheckpointStore,
            "memory-store" => Self::MemoryStore,
            "embedding-provider" => Self::EmbeddingProvider,
            "context-provider" => Self::ContextProvider,
            "context-planner" => Self::ContextPlanner,
            "tool-provider" => Self::ToolProvider,
            "policy-contributor" => Self::PolicyContributor,
            "sandbox-backend" => Self::SandboxBackend,
            "event-sink" => Self::EventSink,
            "task-executor" => Self::TaskExecutor,
            "status-segment" => Self::StatusSegment,
            "palette-source" => Self::PaletteSource,
            "speech-transcriber" => Self::SpeechTranscriber,
            "speech-synthesizer" => Self::SpeechSynthesizer,
            "media-generator" => Self::MediaGenerator,
            other => Self::Other(other.to_string()),
        }
    }
}

impl Serialize for ExtensionCategory {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ExtensionCategory {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        Ok(Self::from_name(&name))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DistributionManifest {
    pub name: String,
    pub version: String,
    pub include_tui: bool,
    pub include_app_server: bool,
    pub include_cli: bool,
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default)]
    pub default_provider: Option<String>,
    #[serde(default)]
    pub default_thread_store: Option<String>,
    #[serde(default)]
    pub config_overrides: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Profile {
    pub id: String,
    pub description: String,
    pub manifest: DistributionManifest,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum CatalogError {
    MissingMetadata {
        crate_name: String,
        manifest_path: Option<String>,
    },
    MalformedMetadata {
        crate_name: String,
        manifest_path: Option<String>,
        message: String,
    },
    Conflict {
        first_id: String,
        second_id: String,
        reason: String,
    },
    CapabilityDisabled {
        extension_id: String,
        capability: String,
    },
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingMetadata {
                crate_name,
                manifest_path,
            } => match manifest_path {
                Some(path) => write!(
                    f,
                    "crate `{crate_name}` has no [package.metadata.roder.distribution] metadata in {path}"
                ),
                None => write!(
                    f,
                    "crate `{crate_name}` has no [package.metadata.roder.distribution] metadata"
                ),
            },
            Self::MalformedMetadata {
                crate_name,
                manifest_path,
                message,
            } => match manifest_path {
                Some(path) => write!(
                    f,
                    "crate `{crate_name}` has malformed distribution metadata in {path}: {message}"
                ),
                None => write!(
                    f,
                    "crate `{crate_name}` has malformed distribution metadata: {message}"
                ),
            },
            Self::Conflict {
                first_id,
                second_id,
                reason,
            } => write!(
                f,
                "distribution entries `{first_id}` and `{second_id}` conflict: {reason}"
            ),
            Self::CapabilityDisabled {
                extension_id,
                capability,
            } => write!(
                f,
                "distribution entry `{extension_id}` requires disabled capability `{capability}`"
            ),
        }
    }
}

impl Error for CatalogError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distribution_entry_round_trips_json() {
        let entry = DistributionEntry {
            id: "openai-responses".to_string(),
            crate_name: "roder-ext-openai-responses".to_string(),
            category: ExtensionCategory::InferenceEngine,
            display_name: "OpenAI Responses".to_string(),
            description: "OpenAI Responses-style inference".to_string(),
            default_in_profiles: vec!["full".to_string(), "openai-only".to_string()],
            required_env: vec!["OPENAI_API_KEY".to_string()],
            optional_env: vec!["OPENAI_BASE_URL".to_string()],
            conflicts_with: vec![],
            required_capabilities: vec![
                "network.api.openai.com".to_string(),
                "secret.read.OPENAI_API_KEY".to_string(),
            ],
            extension_path: "::extension".to_string(),
            docs_url: Some("https://platform.openai.com/docs/api-reference/responses".to_string()),
            extras: serde_json::json!({ "reasoning": true }),
        };

        let encoded = serde_json::to_value(&entry).unwrap();
        assert_eq!(encoded["category"], "inference-engine");
        let decoded: DistributionEntry = serde_json::from_value(encoded).unwrap();

        assert_eq!(decoded, entry);
    }

    #[test]
    fn extension_category_other_remains_extensible() {
        // Novel category names (declared by newer crates) parse into
        // `Other` and round-trip as the same plain string, so older
        // tooling never fails on metadata it has not heard of.
        for name in ["browser-automation", "inference-router"] {
            let decoded: ExtensionCategory =
                serde_json::from_value(serde_json::json!(name)).unwrap();
            assert_eq!(decoded, ExtensionCategory::Other(name.to_string()));
            assert_eq!(decoded.as_str(), name);
            assert_eq!(
                serde_json::to_value(decoded).unwrap(),
                serde_json::json!(name)
            );
        }
        // Known categories still parse into their variants.
        let known: ExtensionCategory =
            serde_json::from_value(serde_json::json!("inference-engine")).unwrap();
        assert_eq!(known, ExtensionCategory::InferenceEngine);
    }

    #[test]
    fn speech_extension_categories_parse_from_metadata() {
        #[derive(Deserialize)]
        struct CategoryFixture {
            category: ExtensionCategory,
        }

        let transcriber: CategoryFixture =
            toml::from_str(r#"category = "speech-transcriber""#).unwrap();
        let synthesizer: CategoryFixture =
            toml::from_str(r#"category = "speech-synthesizer""#).unwrap();

        assert_eq!(transcriber.category, ExtensionCategory::SpeechTranscriber);
        assert_eq!(synthesizer.category, ExtensionCategory::SpeechSynthesizer);
    }

    #[test]
    fn distribution_manifest_and_profile_round_trip() {
        let profile = Profile {
            id: "research-headless".to_string(),
            description: "Headless app-server distribution".to_string(),
            manifest: DistributionManifest {
                name: "research-roder".to_string(),
                version: "0.1.0".to_string(),
                include_tui: false,
                include_app_server: true,
                include_cli: true,
                extensions: vec!["jsonl-thread-store".to_string(), "memory".to_string()],
                default_provider: Some("openai-responses".to_string()),
                default_thread_store: Some("jsonl-thread-store".to_string()),
                config_overrides: serde_json::json!({
                    "subagents": { "max_depth": 1 }
                }),
            },
        };

        let encoded = serde_json::to_string(&profile).unwrap();
        let decoded: Profile = serde_json::from_str(&encoded).unwrap();

        assert_eq!(decoded, profile);
    }

    #[test]
    fn catalog_error_messages_are_actionable() {
        let error = CatalogError::MalformedMetadata {
            crate_name: "roder-ext-test".to_string(),
            manifest_path: Some("crates/roder-ext-test/Cargo.toml".to_string()),
            message: "missing field `display_name`".to_string(),
        };

        let message = error.to_string();
        assert!(message.contains("roder-ext-test"));
        assert!(message.contains("Cargo.toml"));
        assert!(message.contains("display_name"));
    }

    #[test]
    fn distribution_entry_parses_from_cargo_metadata_toml_shape() {
        let toml = r#"
id = "openai-responses"
crate_name = "roder-ext-openai-responses"
category = "inference-engine"
display_name = "OpenAI Responses"
description = "OpenAI Responses-style inference."
default_in_profiles = ["full", "openai-only"]
required_env = ["OPENAI_API_KEY"]
optional_env = ["OPENAI_BASE_URL"]
conflicts_with = []
required_capabilities = ["network.api.openai.com", "secret.read.OPENAI_API_KEY"]
extension_path = "::extension"
docs_url = "https://platform.openai.com/docs/api-reference/responses"
"#;

        let entry: DistributionEntry = toml::from_str(toml).unwrap();

        assert_eq!(entry.id, "openai-responses");
        assert_eq!(entry.category, ExtensionCategory::InferenceEngine);
        assert_eq!(entry.extension_path, "::extension");
    }
}
