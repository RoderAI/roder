use super::*;

impl AppServer {
    pub(super) async fn handle_settings_set_model_context_window(
        &self,
        params: SettingsSetModelContextWindowParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        if params.model_context_window == Some(0) {
            return Err(invalid_params_error(
                "model_context_window must be positive",
            ));
        }
        if self.persist_user_config {
            // Persist first: a write failure must leave the live setting alone.
            roder_config::save_model_context_window(params.model_context_window)
                .map_err(internal_error)?;
        }
        let cfg = self
            .runtime
            .set_model_context_window(params.model_context_window)
            .await
            .map_err(invalid_params)?;
        Ok(serde_json::to_value(SettingsSetModelContextWindowResult {
            model_context_window: cfg.model_context_window,
            config_path: roder_config::config_file_path().display().to_string(),
        })
        .unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::extension::ExtensionRegistryBuilder;
    use roder_api::inference::{AgentInferenceRequest, InferenceEngine, InferenceTurnContext};
    use roder_core::{RuntimeConfig, fake_provider::FakeInferenceEngine};

    struct CatalogEngine(&'static str);

    #[async_trait::async_trait]
    impl InferenceEngine for CatalogEngine {
        fn id(&self) -> String {
            self.0.into()
        }
        fn capabilities(&self) -> roder_api::inference::InferenceCapabilities {
            roder_api::inference::InferenceCapabilities::text_only()
        }
        async fn list_models(
            &self,
            _: InferenceProviderContext<'_>,
        ) -> anyhow::Result<Vec<roder_api::inference::ModelDescriptor>> {
            Ok(roder_api::catalog::models_for_provider(self.0, false))
        }
        async fn stream_turn(
            &self,
            _: InferenceTurnContext<'_>,
            _: AgentInferenceRequest,
        ) -> anyhow::Result<roder_api::inference::InferenceEventStream> {
            unreachable!("catalog-only engine")
        }
    }

    fn server() -> AppServer {
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(FakeInferenceEngine));
        builder.inference_engine(Arc::new(CatalogEngine("openai")));
        builder.inference_engine(Arc::new(CatalogEngine("anthropic")));
        let runtime = Runtime::new(builder.build().unwrap(), RuntimeConfig::default()).unwrap();
        AppServer::new(Arc::new(runtime))
    }

    #[tokio::test]
    async fn context_settings_update_get_catalog_and_reset_without_persistence() {
        let server = server();
        let before = server.provider_descriptors().await;
        let anthropic = before.iter().find(|p| p.id == "anthropic").unwrap();
        let original = anthropic.models[0].context_window;
        let result = server
            .handle_settings_set_model_context_window(SettingsSetModelContextWindowParams {
                model_context_window: Some(200_000),
            })
            .await
            .unwrap();
        assert_eq!(result["model_context_window"], 200_000);
        assert_eq!(
            result["config_path"],
            roder_config::config_file_path().display().to_string()
        );
        assert_eq!(
            server.handle_settings_get().await.unwrap()["model_context_window"],
            200_000
        );
        let providers = server.provider_descriptors().await;
        let openai = providers.iter().find(|p| p.id == "openai").unwrap();
        assert_eq!(
            openai
                .models
                .iter()
                .find(|m| m.id == "gpt-6.2-sol")
                .unwrap()
                .context_window,
            Some(200_000)
        );
        assert_eq!(
            providers
                .iter()
                .find(|p| p.id == "anthropic")
                .unwrap()
                .models[0]
                .context_window,
            original
        );
        let error = server
            .handle_settings_set_model_context_window(SettingsSetModelContextWindowParams {
                model_context_window: Some(0),
            })
            .await
            .unwrap_err();
        assert_eq!(error.code, -32602);
        assert_eq!(error.message, "model_context_window must be positive");
        assert_eq!(
            server.runtime.status().await.model_context_window,
            Some(200_000)
        );
        server
            .handle_settings_set_model_context_window(SettingsSetModelContextWindowParams {
                model_context_window: None,
            })
            .await
            .unwrap();
        assert!(server.handle_settings_get().await.unwrap()["model_context_window"].is_null());
        let providers = server.provider_descriptors().await;
        assert_eq!(
            providers
                .iter()
                .find(|p| p.id == "openai")
                .unwrap()
                .models
                .iter()
                .find(|m| m.id == "gpt-6.2-sol")
                .unwrap()
                .context_window,
            Some(500_000)
        );
    }
}
