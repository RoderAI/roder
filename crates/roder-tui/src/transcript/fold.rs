use std::collections::BTreeMap;

use roder_api::{
    events::ThreadId,
    extension::ExtensionId,
    extension_state::{
        ExtensionStateCodec, ExtensionStateKey, ExtensionStateRecord, ExtensionStoreScope,
    },
};
use serde::{Deserialize, Serialize};

const TRANSCRIPT_FOLD_EXTENSION_ID: &str = "roder-tui/transcript-folds";
const TRANSCRIPT_FOLD_STATE_KEY: &str = "fold-state";
pub const TRANSCRIPT_FOLD_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TranscriptFoldState {
    pub schema_version: u32,
    #[serde(default)]
    pub expanded: BTreeMap<String, bool>,
}

impl TranscriptFoldState {
    pub fn is_expanded(&self, key: &str) -> bool {
        self.expanded.get(key).copied().unwrap_or(false)
    }

    pub fn toggle(&mut self, key: impl Into<String>) {
        let key = key.into();
        let expanded = !self.is_expanded(&key);
        self.expanded.insert(key, expanded);
        self.schema_version = TRANSCRIPT_FOLD_SCHEMA_VERSION;
    }

    pub fn set_expanded(&mut self, key: impl Into<String>, expanded: bool) {
        self.expanded.insert(key.into(), expanded);
        self.schema_version = TRANSCRIPT_FOLD_SCHEMA_VERSION;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptFoldStateCodec {
    thread_id: ThreadId,
}

impl TranscriptFoldStateCodec {
    pub fn thread(thread_id: impl Into<ThreadId>) -> Self {
        Self {
            thread_id: thread_id.into(),
        }
    }

    pub fn encode(&self, state: &TranscriptFoldState) -> anyhow::Result<ExtensionStateRecord> {
        self.encode_state(state)
    }

    pub fn decode(&self, record: &ExtensionStateRecord) -> anyhow::Result<TranscriptFoldState> {
        self.decode_state(record)
    }
}

impl ExtensionStateCodec for TranscriptFoldStateCodec {
    type State = TranscriptFoldState;

    fn extension_id(&self) -> ExtensionId {
        TRANSCRIPT_FOLD_EXTENSION_ID.to_string()
    }

    fn key(&self) -> ExtensionStateKey {
        TRANSCRIPT_FOLD_STATE_KEY.to_string()
    }

    fn scope(&self) -> ExtensionStoreScope {
        ExtensionStoreScope::Thread {
            thread_id: self.thread_id.clone(),
        }
    }

    fn schema_version(&self) -> u32 {
        TRANSCRIPT_FOLD_SCHEMA_VERSION
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fold_state_round_trips_json_and_toggles_by_stable_key() {
        let mut state = TranscriptFoldState::default();
        state.toggle("call_1");
        assert!(state.is_expanded("call_1"));
        state.toggle("call_1");
        assert!(!state.is_expanded("call_1"));

        state.toggle("call_2");
        let encoded = serde_json::to_value(&state).unwrap();
        let decoded: TranscriptFoldState = serde_json::from_value(encoded).unwrap();
        assert!(decoded.is_expanded("call_2"));
    }

    #[test]
    fn fold_state_codec_persists_at_thread_scope() {
        let mut state = TranscriptFoldState::default();
        state.set_expanded("call_1", true);
        state.set_expanded("call_2", false);

        let codec = TranscriptFoldStateCodec::thread("thread-a");
        let record = codec.encode(&state).unwrap();

        assert_eq!(
            record.scope,
            ExtensionStoreScope::Thread {
                thread_id: "thread-a".to_string()
            }
        );
        assert_eq!(record.schema_version, TRANSCRIPT_FOLD_SCHEMA_VERSION);
        assert_eq!(codec.decode(&record).unwrap(), state);
    }
}
