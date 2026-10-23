//! Cursor checkpoints reference content-addressed blobs, not inline messages.
//! Keep both across turns; a conversation id alone does not restore context.
use crate::proto::{proto_field_bytes, proto_field_string, proto_field_varint, proto_message};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use roder_api::transcript::TranscriptItem;
use serde_json::{Value, json};
use std::collections::BTreeMap;

const MAX_STATE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Default)]
pub struct CursorConversationState {
    pub checkpoint: Vec<u8>,
    pub blobs: BTreeMap<Vec<u8>, Vec<u8>>,
}
impl CursorConversationState {
    pub fn from_transcript(
        items: &[TranscriptItem],
        conversation_id: &str,
    ) -> anyhow::Result<Self> {
        for item in items.iter().rev() {
            match item {
                TranscriptItem::ContextCompaction(_) => break,
                TranscriptItem::ProviderMetadata(metadata)
                    if metadata["provider"] == "cursor"
                        && metadata["conversationId"] == conversation_id =>
                {
                    if let Some(state) = metadata.get("cursorState") {
                        return Self::decode(state);
                    }
                }
                _ => {}
            }
        }
        Ok(Self::default())
    }
    fn decode(value: &Value) -> anyhow::Result<Self> {
        let checkpoint = STANDARD.decode(
            value["checkpoint"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Cursor checkpoint is missing"))?,
        )?;
        let mut state = Self {
            checkpoint,
            blobs: BTreeMap::new(),
        };
        if let Some(blobs) = value["blobs"].as_array() {
            for blob in blobs {
                state.put(
                    STANDARD.decode(
                        blob["id"]
                            .as_str()
                            .ok_or_else(|| anyhow::anyhow!("Cursor blob id is missing"))?,
                    )?,
                    STANDARD.decode(
                        blob["value"]
                            .as_str()
                            .ok_or_else(|| anyhow::anyhow!("Cursor blob value is missing"))?,
                    )?,
                )?;
            }
        }
        Ok(state)
    }
    pub fn metadata(&self) -> Value {
        json!({"checkpoint":STANDARD.encode(&self.checkpoint),"blobs":self.blobs.iter().map(|(id,value)|json!({"id":STANDARD.encode(id),"value":STANDARD.encode(value)})).collect::<Vec<_>>()})
    }
    pub fn put(&mut self, id: Vec<u8>, value: Vec<u8>) -> anyhow::Result<()> {
        let size = self.checkpoint.len()
            + self
                .blobs
                .iter()
                .filter(|(key, _)| **key != id)
                .map(|(key, value)| key.len() + value.len())
                .sum::<usize>()
            + id.len()
            + value.len();
        anyhow::ensure!(
            size <= MAX_STATE_BYTES,
            "Cursor conversation state exceeds 64 MiB"
        );
        self.blobs.insert(id, value);
        Ok(())
    }
    pub fn pre_fetched_blobs(&self) -> Vec<Vec<u8>> {
        self.blobs
            .iter()
            .map(|(id, value)| {
                proto_field_bytes(
                    17,
                    proto_message(vec![
                        proto_field_bytes(1, id.clone()),
                        proto_field_bytes(2, value.clone()),
                    ]),
                )
            })
            .collect()
    }
    pub fn get_reply(&self, seq: u64, id: &[u8]) -> Vec<u8> {
        let result = match self.blobs.get(id) {
            Some(value) => proto_field_bytes(1, value.clone()),
            None => proto_field_bytes(
                2,
                proto_message(vec![proto_field_string(1, "Cursor blob not found")]),
            ),
        };
        proto_message(vec![proto_field_bytes(
            3,
            proto_message(vec![
                proto_field_varint(1, seq),
                proto_field_bytes(2, result),
            ]),
        )])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn state_round_trip_preserves_checkpoint_and_binary_blobs() {
        let mut state = CursorConversationState {
            checkpoint: vec![1, 2, 3],
            ..Default::default()
        };
        state.put(vec![9, 8], vec![0, 255, 7]).unwrap();
        let restored = CursorConversationState::decode(&state.metadata()).unwrap();
        assert_eq!(restored.checkpoint, state.checkpoint);
        assert_eq!(restored.blobs, state.blobs);
    }
}

#[cfg(test)]
mod protocol_tests {
    use super::*;
    use crate::proto::{scalar_string, submessage};
    #[test]
    fn get_replies_with_blob_bytes_or_typed_missing_error() {
        let mut state = CursorConversationState::default();
        state.put(vec![1, 2], b"remote content".to_vec()).unwrap();
        let reply = submessage(&state.get_reply(7, &[1, 2]), 3).unwrap();
        let result = submessage(&reply, 2).unwrap();
        assert_eq!(
            submessage(&result, 1).as_deref(),
            Some(b"remote content".as_slice())
        );
        let reply = submessage(&state.get_reply(8, &[9]), 3).unwrap();
        let error = submessage(&submessage(&reply, 2).unwrap(), 2).unwrap();
        assert!(scalar_string(&error, 1).unwrap().contains("not found"));
    }
    #[test]
    fn run_request_restores_checkpoint_and_prefetched_blob_contract() {
        let mut state = CursorConversationState {
            checkpoint: vec![8, 7, 6],
            ..Default::default()
        };
        state.put(vec![1, 2], vec![3, 4]).unwrap();
        let client = crate::proto::encode_agent_client_message(
            "next",
            "composer-2.5",
            "thread",
            "message",
            &[],
            &[],
            None,
            Some(&state),
        );
        let request = submessage(&client, 1).unwrap();
        assert_eq!(submessage(&request, 1), Some(state.checkpoint));
        let prefetched = submessage(&request, 17).unwrap();
        assert_eq!(submessage(&prefetched, 1), Some(vec![1, 2]));
        assert_eq!(submessage(&prefetched, 2), Some(vec![3, 4]));
    }
}
