//! Artifact descriptors for browser outputs (screenshots, recordings,
//! downloads, action traces, extractions).
//!
//! Screenshots/recordings/downloads/traces should be written under Roder
//! artifact roots with redaction metadata; this module defines the shared
//! descriptor. Writers are added by the host-side integration.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChromeArtifactKind {
    Screenshot,
    Recording,
    Download,
    ActionTrace,
    Extraction,
}

/// A browser artifact stored under a Roder artifact root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChromeArtifact {
    pub kind: ChromeArtifactKind,
    /// Relative path under the artifact root.
    pub path: String,
    #[serde(default)]
    pub mime_type: Option<String>,
    /// Origin the artifact was captured from, for redaction auditing.
    #[serde(default)]
    pub origin: Option<String>,
    /// True when sensitive fields were stripped before persistence.
    #[serde(default)]
    pub redacted: bool,
}

impl ChromeArtifact {
    pub fn new(kind: ChromeArtifactKind, path: impl Into<String>) -> Self {
        Self {
            kind,
            path: path.into(),
            mime_type: None,
            origin: None,
            redacted: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_round_trips() {
        let artifact = ChromeArtifact::new(ChromeArtifactKind::Screenshot, "shots/1.png");
        let encoded = serde_json::to_value(&artifact).unwrap();
        assert_eq!(encoded["kind"], "screenshot");
        let decoded: ChromeArtifact = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded, artifact);
    }
}
