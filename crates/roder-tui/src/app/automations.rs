use roder_protocol::{AutomationsListResult, AutomationsStatusResult, JsonRpcRequest};

use super::{AppClient, TuiApp, decode_response};

impl<C> TuiApp<C>
where
    C: AppClient,
{
    pub(super) async fn automations_status(&self) -> anyhow::Result<AutomationsStatusResult> {
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("automations/status")),
                method: "automations/status".to_string(),
                params: None,
            })
            .await;
        decode_response(res)
    }

    pub(super) async fn automations_list(&self) -> anyhow::Result<AutomationsListResult> {
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("automations/list")),
                method: "automations/list".to_string(),
                params: None,
            })
            .await;
        decode_response(res)
    }

    pub(super) async fn show_automations_status(&mut self) {
        let status = match self.automations_status().await {
            Ok(status) => status,
            Err(err) => {
                self.push_event(format!("automations/status failed: {err}"));
                return;
            }
        };
        let list = match self.automations_list().await {
            Ok(list) => list,
            Err(err) => {
                self.push_event(format!("automations/list failed: {err}"));
                return;
            }
        };

        let scheduler = if status.scheduler_enabled {
            "enabled"
        } else {
            "disabled"
        };
        self.push_event(format!(
            "automations: scheduler {scheduler}; {} definitions; active={}, due={}, leased={}",
            list.automations.len(),
            status.active_runs,
            status.due_count,
            status.leased_count
        ));
    }
}
