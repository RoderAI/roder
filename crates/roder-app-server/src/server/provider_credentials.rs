use super::{AppServer, internal_error, invalid_params};
use roder_protocol::{
    JsonRpcError, ProviderClearParams, ProviderClearResult, ProviderConfigureParams,
    ProviderConfigureResult,
};

impl AppServer {
    pub(super) async fn handle_provider_configure(
        &self,
        params: ProviderConfigureParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let provider = roder_api::catalog::normalize_provider_id(params.provider.trim());
        let api_key = params.api_key.trim();
        if provider.is_empty() {
            return Err(invalid_params("provider is required"));
        }
        if api_key.is_empty() {
            return Err(invalid_params("api_key is required"));
        }
        let registry = self.runtime.registry();
        let inference_provider = registry
            .inference_engines
            .iter()
            .any(|engine| engine.id() == provider);
        let jev_tool_provider = provider == "jev"
            && registry
                .tools
                .iter()
                .any(|contributor| contributor.id() == provider);
        if !inference_provider && !jev_tool_provider {
            return Err(invalid_params(format!("unknown provider {provider:?}")));
        }
        if !self.persist_user_config {
            return Err(internal_error(
                "provider API key persistence is disabled for this app-server",
            ));
        }
        roder_config::save_provider_api_key(&provider, api_key).map_err(internal_error)?;
        // Synthetic web search shares its inference provider key.
        if provider == "synthetic" {
            let _ = roder_config::save_web_search_provider_enabled("synthetic", true);
        }
        Ok(serde_json::to_value(ProviderConfigureResult {
            provider,
            authenticated: true,
        })
        .unwrap())
    }

    pub(super) async fn handle_provider_clear(
        &self,
        params: ProviderClearParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let provider = roder_api::catalog::normalize_provider_id(params.provider.trim());
        if provider.is_empty() {
            return Err(invalid_params("provider is required"));
        }
        if !self.persist_user_config {
            return Err(internal_error(
                "provider API key persistence is disabled for this app-server",
            ));
        }
        roder_config::delete_provider_api_key(&provider).map_err(internal_error)?;
        Ok(serde_json::to_value(ProviderClearResult { provider }).unwrap())
    }
}
