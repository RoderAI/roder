use roder_app_server::AppClient;
use roder_protocol::{JsonRpcRequest, ThreadCompactParams, ThreadCompactResult};

use super::{TuiApp, decode_response, slash_command_suffix};

impl<C> TuiApp<C>
where
    C: AppClient,
{
    pub(super) async fn run_compact_slash_command(&mut self, args: &str) {
        let turn_id = self
            .active_turn_id
            .clone()
            .unwrap_or_else(|| "slash-compact".to_string());
        let preserve_hint = args.trim();
        let preserve_hint = (!preserve_hint.is_empty()).then(|| preserve_hint.to_string());
        match thread_compact(
            &self.client,
            ThreadCompactParams {
                thread_id: self.thread_id.clone(),
                turn_id,
                preserve_hint,
            },
        )
        .await
        {
            Ok(result) => {
                if result.compacted {
                    self.timeline.push_system(format!(
                        "Context compacted ({} -> {} est. tokens).",
                        result.estimated_tokens_before, result.estimated_tokens_after
                    ));
                } else {
                    let reason = result
                        .reason
                        .unwrap_or_else(|| "nothing to compact".to_string());
                    self.timeline
                        .push_system(format!("Context compaction skipped: {reason}."));
                }
            }
            Err(err) => self.record_error(format!("thread/compact failed: {err}")),
        }
        self.push_event(format!(
            "slash command: /compact{}",
            slash_command_suffix(args)
        ));
    }
}

async fn thread_compact<C: AppClient>(
    client: &C,
    params: ThreadCompactParams,
) -> anyhow::Result<ThreadCompactResult> {
    let res = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("thread/compact")),
            method: "thread/compact".to_string(),
            params: Some(serde_json::to_value(params)?),
        })
        .await;
    decode_response(res)
}