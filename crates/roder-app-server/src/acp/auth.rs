use agent_client_protocol_schema as acp;
use async_trait::async_trait;
use roder_protocol::JsonRpcError;

use super::{AcpAdapter, decode_params, invalid_params, method_not_found};
use crate::client::AppClient;

#[async_trait]
pub trait AcpAuthentication: Send + Sync {
    fn methods(&self) -> Vec<acp::AuthMethod>;
    async fn is_authenticated(&self) -> anyhow::Result<bool>;
    async fn authenticate(&self, method_id: &str) -> anyhow::Result<()>;
}

impl<C: AppClient> AcpAdapter<C> {
    pub(super) async fn require_authentication(&self) -> Result<(), JsonRpcError> {
        if let Some(auth) = &self.authentication {
            if !auth
                .is_authenticated()
                .await
                .map_err(authentication_error)?
            {
                return Err(JsonRpcError {
                    code: -32000,
                    message: "Authentication required".into(),
                    data: None,
                });
            }
        }
        Ok(())
    }

    pub(super) async fn handle_authenticate(
        &self,
        params: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let request: acp::AuthenticateRequest = decode_params(params)?;
        let auth = self
            .authentication
            .as_ref()
            .ok_or_else(|| method_not_found("authenticate".into()))?;
        let method_id = request.method_id.to_string();
        if !auth
            .methods()
            .iter()
            .any(|method| method.id().to_string() == method_id)
        {
            return Err(invalid_params(format!(
                "unknown authentication method {method_id:?}"
            )));
        }
        auth.authenticate(&method_id)
            .await
            .map_err(authentication_error)?;
        serde_json::to_value(acp::AuthenticateResponse::new()).map_err(authentication_error)
    }
}

fn authentication_error(error: impl std::fmt::Display) -> JsonRpcError {
    JsonRpcError {
        code: -32603,
        message: format!("Authentication failed: {error}"),
        data: None,
    }
}
