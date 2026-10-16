use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::SandboxPort;

use super::{ForkError, types::validate_name};

/// Non-secret runtime pins captured before snapshot creation and persisted with
/// the caller's checkpoint. This is configuration evidence, not authorization.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ForkRuntime {
    pub image: String,
    pub memory_mb: u32,
    pub region: String,
    /// Actual node generation from the sandbox GET, e.g. mk3.0 or mk3.1.
    pub node_generation: String,
    pub ports: Vec<SandboxPort>,
}

impl ForkRuntime {
    pub(super) fn validate(&self) -> Result<(), ForkError> {
        if self.image.is_empty()
            || self.image.len() > 4096
            || self.image.chars().any(char::is_control)
            || self.memory_mb == 0
            || !matches!(self.node_generation.as_str(), "mk3.0" | "mk3.1")
        {
            return Err(ForkError::rejected("unsupported fork runtime pins"));
        }
        validate_name(&self.region)?;
        crate::ports::parse_ports(&serde_json::json!({"ports":self.ports}))
            .map_err(|_| ForkError::rejected("unsupported fork ports"))?;
        Ok(())
    }

    pub(super) fn from_sandbox(spec: &Value, node_generation: &str) -> Result<Self, ForkError> {
        validate_attachments(spec)?;
        let runtime = &spec["runtime"];
        let pins = Self {
            image: string(&runtime["image"])?.into(),
            memory_mb: runtime["memory"]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| ForkError::rejected("invalid sandbox memory"))?,
            region: string(&spec["region"])?.into(),
            node_generation: node_generation.into(),
            ports: crate::ports::parse_ports(runtime)
                .map_err(|_| ForkError::rejected("invalid sandbox ports"))?,
        };
        pins.validate()?;
        Ok(pins)
    }

    pub(super) fn matches(&self, other: &Self) -> bool {
        self.image == other.image
            && self.memory_mb == other.memory_mb
            && self.region == other.region
            && self.node_generation == other.node_generation
            && self.ports.len() == other.ports.len()
            && self.ports.iter().all(|port| other.ports.contains(port))
    }
}

fn string(value: &Value) -> Result<&str, ForkError> {
    value
        .as_str()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| ForkError::rejected("missing sandbox runtime pin"))
}

/// This pilot supports only public shared networking and no attached storage.
/// Reject unknown networking rather than silently dropping it from the pins.
pub(super) fn validate_attachments(spec: &Value) -> Result<(), ForkError> {
    for value in [
        &spec["volumes"],
        &spec["mounts"],
        &spec["runtime"]["volumes"],
        &spec["runtime"]["mounts"],
    ] {
        if !value.is_null() && !value.as_array().is_some_and(Vec::is_empty) {
            return Err(ForkError::rejected(
                "fork storage attachments are unsupported",
            ));
        }
    }
    if !spec["vpc"].is_null() && spec["vpc"] != "" {
        return Err(ForkError::rejected("fork VPC attachments are unsupported"));
    }
    let network = &spec["network"];
    if network.is_null() {
        return Ok(());
    }
    let Some(network) = network.as_object() else {
        return Err(ForkError::rejected(
            "unsupported fork network configuration",
        ));
    };
    if network.keys().any(|key| key != "egress") {
        return Err(ForkError::rejected(
            "fork network attachments are unsupported",
        ));
    }
    if let Some(egress) = network.get("egress").filter(|value| !value.is_null()) {
        let valid = egress.as_object().is_some_and(|egress| {
            egress.keys().all(|key| key == "mode")
                && egress.get("mode").is_none_or(|mode| mode == "shared")
        });
        if !valid {
            return Err(ForkError::rejected("fork egress assignment is unsupported"));
        }
    }
    Ok(())
}
