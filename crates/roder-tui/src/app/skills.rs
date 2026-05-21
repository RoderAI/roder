use roder_api::skills::{SkillExposure, SkillSelector};
use roder_protocol::{
    JsonRpcRequest, SkillsListResult, SkillsSetEnabledParams, SkillsSetExposureParams,
    SkillsUpdateResult,
};

use super::{AppClient, TuiApp, decode_response};

impl<C> TuiApp<C>
where
    C: AppClient,
{
    pub(super) async fn skills_list(&self) -> anyhow::Result<SkillsListResult> {
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("skills/list")),
                method: "skills/list".to_string(),
                params: None,
            })
            .await;
        decode_response(res)
    }

    pub(super) async fn set_skill_enabled(&mut self, selector: SkillSelector, enabled: bool) {
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("skills/setEnabled")),
                method: "skills/setEnabled".to_string(),
                params: Some(
                    serde_json::to_value(SkillsSetEnabledParams { selector, enabled }).unwrap(),
                ),
            })
            .await;
        match decode_response::<SkillsUpdateResult>(res) {
            Ok(result) => {
                self.push_event(format!(
                    "skills: updated enabled state ({} skills)",
                    result.skills.len()
                ));
            }
            Err(err) => self.push_event(format!("skills/setEnabled failed: {err}")),
        }
    }

    pub(super) async fn set_skill_exposure(
        &mut self,
        selector: SkillSelector,
        exposure: SkillExposure,
    ) {
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("skills/setExposure")),
                method: "skills/setExposure".to_string(),
                params: Some(
                    serde_json::to_value(SkillsSetExposureParams { selector, exposure }).unwrap(),
                ),
            })
            .await;
        match decode_response::<SkillsUpdateResult>(res) {
            Ok(result) => {
                self.push_event(format!(
                    "skills: updated exposure state ({} skills)",
                    result.skills.len()
                ));
            }
            Err(err) => self.push_event(format!("skills/setExposure failed: {err}")),
        }
    }
}
