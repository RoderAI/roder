use serde::{Deserialize, Deserializer, de::IgnoredAny};

use crate::SandboxPort;

use super::ForkRuntime;

/// Non-secret configuration carried by a provider snapshot. Other provider
/// fields are discarded. Attached volumes are counted without retaining names.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotSpec {
    pub image: String,
    pub memory: u32,
    pub region: String,
    pub generation: String,
    #[serde(default)]
    pub ports: Vec<SandboxPort>,
    #[serde(default, rename = "volumes", deserialize_with = "count_volumes")]
    volume_count: usize,
}

fn count_volumes<'de, D: Deserializer<'de>>(deserializer: D) -> Result<usize, D::Error> {
    Option::<Vec<IgnoredAny>>::deserialize(deserializer)
        .map(|volumes| volumes.map_or(0, |v| v.len()))
}

impl SnapshotSpec {
    pub(super) fn matches(&self, expected: &ForkRuntime) -> bool {
        self.image == expected.image
            && self.memory == expected.memory_mb
            && self.region == expected.region
            && self.generation == "mk3"
            && expected.node_generation.starts_with("mk3.")
            && self.volume_count == 0
            && self.ports.len() == expected.ports.len()
            && self.ports.iter().all(|port| expected.ports.contains(port))
            && expected.ports.iter().all(|port| self.ports.contains(port))
    }
}
