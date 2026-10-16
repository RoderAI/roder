use std::{fmt, time::Duration};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::Redacted;

/// Whether a failed mutation can already have created or deleted a resource.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// No request was dispatched, or the provider definitively rejected it.
    Rejected,
    /// Reconcile the recorded identity before making another mutation.
    Unknown,
}

/// Sanitized failure: provider bodies, credentials and request URLs are omitted.
pub struct ForkError {
    pub outcome: Outcome,
    pub status: Option<u16>,
    /// Server-requested delay in seconds, when Retry-After is an integer.
    pub retry_after: Option<Duration>,
    pub(crate) message: &'static str,
    pub(crate) fork_receipt: Option<ForkReceipt>,
}

impl fmt::Debug for ForkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ForkError")
            .field("outcome", &self.outcome)
            .field("status", &self.status)
            .field("retry_after", &self.retry_after)
            .field("message", &self.message)
            .field("has_fork_receipt", &self.fork_receipt.is_some())
            .finish()
    }
}

impl ForkError {
    /// Preserve an unexpected snapshot ID for reconciliation. This is an
    /// untrusted provider hint, never authority to delete or adopt a resource.
    /// Receipt values are deliberately omitted from error Debug/Display.
    pub fn fork_receipt(&self) -> Option<&ForkReceipt> {
        self.fork_receipt.as_ref()
    }

    pub(crate) fn rejected(message: &'static str) -> Self {
        Self {
            outcome: Outcome::Rejected,
            status: None,
            retry_after: None,
            message,
            fork_receipt: None,
        }
    }

    pub(crate) fn unknown(message: &'static str) -> Self {
        Self {
            outcome: Outcome::Unknown,
            ..Self::rejected(message)
        }
    }
}

impl fmt::Display for ForkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} (outcome: {:?}, status: {:?})",
            self.message, self.outcome, self.status
        )
    }
}

impl std::error::Error for ForkError {}

/// Snapshot receipt returned by the workspace control plane.
/// Persist the opaque ID; names are only unique within a source sandbox.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub id: String,
    pub name: String,
    pub workspace: String,
    pub status: String,
    pub source: SnapshotSource,
    #[serde(default)]
    pub spec: Option<super::SnapshotSpec>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SnapshotSource {
    pub name: String,
    pub kind: String,
    #[serde(default)]
    pub deleted: bool,
}

impl Snapshot {
    pub fn is_ready(&self) -> bool {
        self.status.eq_ignore_ascii_case("ready")
    }

    pub(crate) fn matches(&self, workspace: &str, source: &str, name: &str) -> bool {
        !self.id.is_empty()
            && self.workspace == workspace
            && self.name == name
            && self.source.name == source
            && self.source.kind == "sandbox"
    }
}

/// A fork was accepted, but is not yet ready or authorized for runner adoption.
/// It intentionally carries no inherited externalId as child ownership evidence.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkReceipt {
    pub name: String,
    #[serde(rename = "type")]
    pub target_type: String,
    pub snapshot_id: Option<String>,
}

/// DELETE acceptance is not proof that a resource is gone. Poll GET for absence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeleteReceipt {
    Accepted,
    Absent,
}

/// A deterministic child identity and fresh lifetime chosen before dispatch.
/// Store the allocation intent durably before calling the client. The marker is
/// an additional identity check, not authorization to bind an inherited browser.
#[derive(Clone, Debug)]
pub struct ForkRequest {
    pub(crate) target: String,
    pub(crate) marker_name: String,
    pub(crate) marker_value: Redacted,
    pub(crate) max_age: Duration,
}

impl ForkRequest {
    pub fn new(
        target: impl Into<String>,
        marker_name: impl Into<String>,
        marker_value: Redacted,
        max_age: Duration,
    ) -> Result<Self, ForkError> {
        let request = Self {
            target: target.into(),
            marker_name: marker_name.into(),
            marker_value,
            max_age,
        };
        validate_name(&request.target)?;
        if request.marker_name.is_empty()
            || request.marker_name.len() > 128
            || !request
                .marker_name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
            || request.marker_name.as_bytes()[0].is_ascii_digit()
            || request.marker_value.is_empty()
            || request.marker_value.as_str().len() > 1024
            || max_age.as_secs() == 0
            || max_age > Duration::from_secs(86400)
        {
            return Err(ForkError::rejected("invalid fork marker or lifetime"));
        }
        Ok(request)
    }

    pub fn target(&self) -> &str {
        &self.target
    }

    pub(crate) fn body(&self) -> Value {
        json!({ "targetType": "sandbox", "targetName": self.target,
            "envs": [{"name":self.marker_name,"value":self.marker_value.as_str(),"secret":false}],
            "lifecycle":{"expirationPolicies":[{"type":"ttl-max-age","value":format!("{}s",self.max_age.as_secs()),"action":"delete"}]}})
    }
}

pub(crate) fn validate_name(name: &str) -> Result<(), ForkError> {
    if name.is_empty()
        || name.len() > 63
        || !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        || name.starts_with('-')
        || name.ends_with('-')
    {
        return Err(ForkError::rejected("invalid provider resource name"));
    }
    Ok(())
}

pub(crate) fn validate_id(id: &str) -> Result<(), ForkError> {
    if id.is_empty()
        || id.len() > 256
        || id == "."
        || id == ".."
        || id.chars().any(char::is_control)
    {
        return Err(ForkError::rejected("invalid snapshot ID"));
    }
    Ok(())
}
