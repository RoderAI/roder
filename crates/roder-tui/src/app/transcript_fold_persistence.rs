use roder_api::events::ThreadId;
use roder_api::extension::{ExtensionStateCodec, ExtensionStoreScope};
use roder_api::session::ThreadSnapshot;
use roder_app_server::LocalAppClient;
use roder_protocol::{ExtensionStateSetParams, ExtensionStateSetResult, JsonRpcRequest};

use super::decode_response;
use crate::transcript::TranscriptFoldState;

pub(super) fn from_snapshot(
    thread_id: &ThreadId,
    snapshot: &ThreadSnapshot,
) -> anyhow::Result<Option<TranscriptFoldState>> {
    let key = TranscriptFoldState::state_key(ExtensionStoreScope::Thread {
        thread_id: thread_id.clone(),
    });
    let Some(record) = snapshot
        .extension_state
        .iter()
        .find(|record| record.key == key)
        .cloned()
    else {
        return Ok(None);
    };
    Ok(Some(TranscriptFoldState::decode_state(record)?))
}

pub(super) async fn save(
    client: &LocalAppClient,
    thread_id: &ThreadId,
    state: &TranscriptFoldState,
) -> anyhow::Result<bool> {
    let record = state.encode_state(ExtensionStoreScope::Thread {
        thread_id: thread_id.clone(),
    })?;
    let res = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("extension_state/set")),
            method: "extension_state/set".to_string(),
            params: Some(serde_json::to_value(ExtensionStateSetParams { record })?),
        })
        .await;
    Ok(decode_response::<ExtensionStateSetResult>(res)?.saved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::session::ThreadSnapshot;

    #[test]
    fn snapshot_state_decodes_thread_scoped_fold_state() {
        let thread_id = "thread-1".to_string();
        let mut state = TranscriptFoldState::default();
        state.toggle_tool_call("call-1");
        let record = state
            .encode_state(ExtensionStoreScope::Thread {
                thread_id: thread_id.clone(),
            })
            .unwrap();
        let snapshot = ThreadSnapshot {
            extension_state: vec![record],
            ..ThreadSnapshot::default()
        };

        let decoded = from_snapshot(&thread_id, &snapshot).unwrap().unwrap();

        assert!(!decoded.is_tool_call_expanded("call-1"));
    }
}
