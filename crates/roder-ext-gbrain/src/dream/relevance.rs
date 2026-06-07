use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceCardScoreInput {
    pub recency: f32,
    pub source_trust: f32,
    pub query_frequency: f32,
    pub eval_failure_feedback: f32,
    pub temporal_specificity: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityIdInput {
    pub scope_id: String,
    pub algorithm_version: String,
    pub node_ids: Vec<String>,
}

pub fn score_evidence_card(input: EvidenceCardScoreInput) -> f32 {
    let score = clamp(input.recency) * 0.20
        + clamp(input.source_trust) * 0.30
        + clamp(input.query_frequency) * 0.15
        + clamp(input.eval_failure_feedback) * 0.20
        + clamp(input.temporal_specificity) * 0.15;
    (score * 10_000.0).round() / 10_000.0
}

pub fn stable_community_id(input: CommunityIdInput) -> String {
    let mut nodes = input.node_ids;
    nodes.sort();
    nodes.dedup();

    let mut hasher = Sha256::new();
    hasher.update(input.scope_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(input.algorithm_version.as_bytes());
    for node in nodes {
        hasher.update(b"\0");
        hasher.update(node.as_bytes());
    }
    let digest = hasher.finalize();
    format!(
        "community:{}:{}",
        sanitize(&input.scope_id),
        hex_prefix(&digest, 12)
    )
}

fn clamp(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

fn sanitize(input: &str) -> String {
    input
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

fn hex_prefix(bytes: &[u8], len: usize) -> String {
    bytes
        .iter()
        .flat_map(|byte| [byte >> 4, byte & 0x0f])
        .take(len)
        .map(|nibble| char::from_digit(nibble as u32, 16).unwrap())
        .collect()
}
