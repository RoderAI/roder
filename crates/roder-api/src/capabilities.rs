use serde::{Deserialize, Serialize};

pub type CapabilityId = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityDecision {
    Requested,
    Granted,
    Denied,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityRequest {
    pub id: CapabilityId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl CapabilityRequest {
    pub fn new(id: impl Into<CapabilityId>) -> Self {
        Self {
            id: id.into(),
            reason: None,
        }
    }

    pub fn with_reason(id: impl Into<CapabilityId>, reason: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            reason: Some(reason.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityGrant {
    pub id: CapabilityId,
}

impl CapabilityGrant {
    pub fn new(id: impl Into<CapabilityId>) -> Self {
        Self { id: id.into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityDenial {
    pub id: CapabilityId,
    pub reason: String,
}

impl CapabilityDenial {
    pub fn new(id: impl Into<CapabilityId>, reason: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            reason: reason.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityStatus {
    pub id: CapabilityId,
    pub decision: CapabilityDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}
