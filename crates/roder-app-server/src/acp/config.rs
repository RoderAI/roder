use super::*;
use roder_protocol::{
    ModelListResult, ModelSelectChoice, ModelSelectParams, ModelSelectResult, ThreadReadParams,
    ThreadReadResult, ThreadStateResult,
};

impl<C: AppClient> AcpAdapter<C> {
    pub(super) async fn session_configuration(
        &self,
        id: &str,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let read: ThreadReadResult = self
            .call(
                "thread/read",
                serde_json::to_value(ThreadReadParams {
                    thread_id: id.into(),
                    include_turns: false,
                })
                .map_err(internal_error)?,
            )
            .await?;
        let thread = read
            .thread
            .ok_or_else(|| invalid_params("session not found"))?;
        let models: ModelListResult = self.call("model/list", serde_json::json!({})).await?;
        let state: ThreadStateResult = self
            .call("thread/state", serde_json::json!({"threadId":id}))
            .await?;
        let current = format!("{}/{}", thread.model_provider, thread.model);
        let mut options = models.models.into_iter().map(|model| serde_json::json!({
            "value":format!("{}/{}", model.model_provider, model.id), "name":format!("{} ({})",model.name,model.model_provider)
        })).collect::<Vec<_>>();
        if !options.iter().any(|option| option["value"] == current) {
            options.push(serde_json::json!({"value":current,"name":current}));
        }
        let mode = serde_json::to_value(state.mode).map_err(internal_error)?;
        Ok(serde_json::json!({
            "modes":{"currentModeId":mode,"availableModes":[
                {"id":"default","name":"Default","description":"Ask before protected tools"},
                {"id":"accept_all","name":"Accept edits","description":"Approve coding tools automatically"},
                {"id":"plan","name":"Plan","description":"Read-only planning"},
                {"id":"bypass","name":"Full access","description":"Bypass built-in tool approvals"}
            ]},
            "configOptions":[{"id":"model","name":"Model","category":"model","type":"select","currentValue":current,"options":options}]
        }))
    }

    pub(super) async fn handle_session_config<P: AcpClientPeer>(
        &self,
        params: Option<serde_json::Value>,
        peer: &P,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let _admission = self.admission.lock().await;
        let request: acp::SetSessionConfigOptionRequest = decode_params(params)?;
        let id = request.session_id.to_string();
        let session = self.active_session(&id).await?;
        let _idle = session
            .active
            .try_lock()
            .map_err(|_| invalid_params("session has active work"))?;
        if request.config_id.to_string() != "model" {
            return Err(invalid_params("unknown session config option"));
        }
        let value = request.value.to_string();
        let config = self.session_configuration(&id).await?;
        if !config["configOptions"][0]["options"]
            .as_array()
            .expect("model choices")
            .iter()
            .any(|option| option["value"] == value)
        {
            return Err(invalid_params("unknown model selection"));
        }
        let (provider, model) = value
            .split_once('/')
            .ok_or_else(|| invalid_params("model must be provider/model"))?;
        let _: ModelSelectResult = self
            .call(
                "model/select",
                serde_json::to_value(ModelSelectParams {
                    thread_id: Some(id.clone()),
                    selection: ModelSelectChoice::Manual {
                        provider: provider.into(),
                        model: Some(model.into()),
                        reasoning: None,
                    },
                })
                .map_err(internal_error)?,
            )
            .await?;
        let config = self.session_configuration(&id).await?;
        let options = config["configOptions"].clone();
        self.send_update(
            peer,
            &id,
            serde_json::json!({"sessionUpdate":"config_option_update","configOptions":options}),
        )
        .await?;
        Ok(serde_json::json!({"configOptions":options}))
    }

    pub(super) async fn handle_session_mode<P: AcpClientPeer>(
        &self,
        params: Option<serde_json::Value>,
        peer: &P,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let _admission = self.admission.lock().await;
        let request: acp::SetSessionModeRequest = decode_params(params)?;
        let id = request.session_id.to_string();
        let session = self.active_session(&id).await?;
        let _idle = session
            .active
            .try_lock()
            .map_err(|_| invalid_params("session has active work"))?;
        let mode: roder_api::policy_mode::PolicyMode =
            serde_json::from_value(serde_json::json!(request.mode_id.to_string()))
                .map_err(invalid_params)?;
        let result: roder_protocol::ThreadSetModeResult = self
            .call(
                "thread/set_mode",
                serde_json::json!({"threadId":id,"mode":mode}),
            )
            .await?;
        self.send_update(
            peer,
            &id,
            serde_json::json!({"sessionUpdate":"current_mode_update","currentModeId":result.mode}),
        )
        .await?;
        Ok(serde_json::json!({}))
    }

    pub(super) async fn active_session(&self, id: &str) -> Result<AcpSession, JsonRpcError> {
        self.sessions
            .lock()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| invalid_params("session is not active"))
    }

    pub(super) async fn send_update<P: AcpClientPeer>(
        &self,
        peer: &P,
        id: &str,
        update: serde_json::Value,
    ) -> Result<(), JsonRpcError> {
        peer.send_notification(JsonRpcNotification {
            jsonrpc: "2.0".into(),
            method: "session/update".into(),
            params: serde_json::json!({"sessionId":id,"update":update}),
        })
        .await
        .map_err(internal_error)
    }
}

pub(super) fn host_instructions(meta: Option<&acp::Meta>) -> Result<Option<String>, JsonRpcError> {
    let Some(value) = meta
        .and_then(|meta| meta.get("_paseo"))
        .and_then(|meta| meta.get("systemPrompt"))
    else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let text = value
        .as_str()
        .ok_or_else(|| invalid_params("_paseo.systemPrompt must be text"))?;
    Ok(Some(text.into()))
}
