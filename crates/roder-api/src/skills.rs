use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::events::{ThreadId, TurnId};

pub type SkillId = String;
pub type SkillName = String;
pub type SkillCanonicalPath = String;
pub type SkillDiagnosticMessage = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "camelCase")]
pub enum SkillSource {
    Workspace,
    User,
    Plugin { plugin_id: String },
    Imported { import_id: String },
    BuiltIn,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SkillExposure {
    Global,
    DirectOnly,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SkillActivationState {
    Enabled,
    Disabled,
    Experimental,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "camelCase")]
pub enum SkillSelector {
    Name { name: SkillName },
    Path { path: SkillCanonicalPath },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SkillActivationReason {
    DirectInvocation,
    FeatureBinding,
    GlobalIndex,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FeatureSkillBinding {
    pub feature_id: String,
    pub skill_selector: SkillSelector,
    pub required: bool,
    pub activation_reason: SkillActivationReason,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillAgentMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interface: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub policies: Vec<String>,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub raw: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillDescriptor {
    pub id: SkillId,
    pub name: SkillName,
    pub canonical_path: SkillCanonicalPath,
    pub source: SkillSource,
    pub exposure: SkillExposure,
    pub activation: SkillActivationState,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_description: Option<String>,
    #[serde(default)]
    pub experimental: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<SkillDiagnosticMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_metadata: Option<SkillAgentMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Skill {
    pub descriptor: SkillDescriptor,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillsCatalogLoaded {
    pub descriptors: Vec<SkillDescriptor>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<SkillDiagnosticMessage>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillConfigApplied {
    pub descriptor: SkillDescriptor,
    pub previous_activation: SkillActivationState,
    pub activation: SkillActivationState,
    pub previous_exposure: SkillExposure,
    pub exposure: SkillExposure,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<SkillDiagnosticMessage>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillActivationResolved {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub selector: SkillSelector,
    pub activation_reason: SkillActivationReason,
    pub activated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub descriptor: Option<SkillDescriptor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<SkillDiagnosticMessage>,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillIndexRendered {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub rendered_count: u64,
    pub hidden_count: u64,
    pub estimated_tokens: u32,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillInvoked {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub selector: SkillSelector,
    pub descriptor: SkillDescriptor,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillAutoActivated {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub feature_id: String,
    pub descriptor: SkillDescriptor,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillSkipped {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub selector: SkillSelector,
    pub reason: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_descriptor_serializes_source_and_exposure() {
        let descriptor = SkillDescriptor {
            id: "builtin:commit".to_string(),
            name: "commit".to_string(),
            canonical_path: "roder-builtin://commit/SKILL.md".to_string(),
            source: SkillSource::BuiltIn,
            exposure: SkillExposure::DirectOnly,
            activation: SkillActivationState::Enabled,
            description: "Commit staged changes safely.".to_string(),
            short_description: Some("Commit safely".to_string()),
            experimental: false,
            diagnostics: Vec::new(),
            agent_metadata: Some(SkillAgentMetadata {
                interface: Some("openai".to_string()),
                dependencies: vec!["git".to_string()],
                policies: vec!["do-not-stage-unrequested-files".to_string()],
                raw: serde_json::json!({ "interface": "openai" }),
            }),
        };

        let value = serde_json::to_value(&descriptor).unwrap();
        assert_eq!(value["canonicalPath"], "roder-builtin://commit/SKILL.md");
        assert_eq!(value["source"], "builtIn");
        assert_eq!(value["exposure"], "direct_only");
        assert_eq!(value["agentMetadata"]["dependencies"][0], "git");
        let round_trip: SkillDescriptor = serde_json::from_value(value).unwrap();
        assert_eq!(round_trip, descriptor);
    }

    #[test]
    fn feature_skill_binding_targets_name_or_path() {
        let binding = FeatureSkillBinding {
            feature_id: "command:commit".to_string(),
            skill_selector: SkillSelector::Name {
                name: "commit".to_string(),
            },
            required: true,
            activation_reason: SkillActivationReason::FeatureBinding,
        };

        let value = serde_json::to_value(binding).unwrap();
        assert_eq!(value["skillSelector"]["name"]["name"], "commit");
        assert_eq!(value["activationReason"], "featureBinding");
    }

    #[test]
    fn skill_events_round_trip_public_shapes() {
        let descriptor = SkillDescriptor {
            id: "builtin:commit".to_string(),
            name: "commit".to_string(),
            canonical_path: "roder-builtin://commit/SKILL.md".to_string(),
            source: SkillSource::BuiltIn,
            exposure: SkillExposure::DirectOnly,
            activation: SkillActivationState::Enabled,
            description: "Commit staged changes safely.".to_string(),
            short_description: Some("Commit safely".to_string()),
            experimental: false,
            diagnostics: Vec::new(),
            agent_metadata: None,
        };
        let event = SkillActivationResolved {
            thread_id: "thread-a".to_string(),
            turn_id: "turn-a".to_string(),
            selector: SkillSelector::Name {
                name: "commit".to_string(),
            },
            activation_reason: SkillActivationReason::DirectInvocation,
            activated: true,
            descriptor: Some(descriptor),
            diagnostic: None,
            timestamp: OffsetDateTime::UNIX_EPOCH,
        };

        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(value["timestamp"], "1970-01-01T00:00:00Z");
        assert_eq!(value["selector"]["name"]["name"], "commit");
        let round_trip: SkillActivationResolved = serde_json::from_value(value).unwrap();
        assert!(round_trip.activated);
    }
}
