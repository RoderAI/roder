use super::*;
use roder_api::inference::{
    InferenceCapabilities, ModelDescriptor, ProviderAuthType, ReasoningEffortDescriptor,
};
use roder_api::inference_routing::ModelSelectionMode;
use roder_protocol::{
    Model, ModelListResult, ModelSelectResult, ProviderDescriptor, ProviderSelectResult,
    ProvidersListResult,
};

fn backend_name(id: &str) -> &str {
    match id {
        "codex" => "Codex",
        "antigravity" => "Antigravity",
        other => other,
    }
}

impl AgentBackendBridge {
    async fn catalog(
        &self,
    ) -> anyhow::Result<(Vec<roder_api::backend::BackendModel>, String, String)> {
        let models = self.backend.list_models(std::env::current_dir()?).await?;
        let selected = self.selected_model.read().await.clone();
        let default = models
            .iter()
            .find(|model| model.is_default)
            .or_else(|| models.first());
        let model = selected
            .as_ref()
            .map(|(model, _)| model.clone())
            .or_else(|| default.map(|model| model.id.clone()))
            .unwrap_or_default();
        let reasoning = selected
            .map(|(_, reasoning)| reasoning)
            .or_else(|| default.map(|model| model.default_reasoning.clone()))
            .unwrap_or_else(|| "medium".into());
        Ok((models, model, reasoning))
    }

    pub(super) async fn initialize(&self) -> anyhow::Result<Value> {
        let (_, model, _) = self.catalog().await?;
        Ok(
            json!({"provider":self.backend.id(),"model":model,"cwd":std::env::current_dir()?.display().to_string()}),
        )
    }

    pub(super) async fn models(&self) -> anyhow::Result<Value> {
        let (models, active, _) = self.catalog().await?;
        Ok(serde_json::to_value(ModelListResult {
            models: models
                .into_iter()
                .map(|model| Model {
                    is_default: model.id == active,
                    id: model.id,
                    name: model.name,
                    model_provider: self.backend.id().into(),
                    default_reasoning_effort: Some(model.default_reasoning),
                    reasoning_efforts: model.reasoning_efforts,
                })
                .collect(),
        })?)
    }

    pub(super) async fn providers(&self) -> anyhow::Result<Value> {
        let (models, active_model, active_reasoning) = self.catalog().await?;
        let provider = self.backend.id().to_owned();
        Ok(serde_json::to_value(ProvidersListResult {
            active_provider: provider.clone(),
            active_model: active_model.clone(),
            active_reasoning: active_reasoning.clone(),
            selection_mode: Some(ModelSelectionMode::manual(
                provider.clone(),
                active_model,
                Some(active_reasoning),
            )),
            routing_options: vec![],
            providers: vec![ProviderDescriptor {
                id: provider,
                name: backend_name(self.backend.id()).into(),
                description: Some(format!("{} agent backend", backend_name(self.backend.id()))),
                auth_type: ProviderAuthType::None,
                auth_label: None,
                authenticated: true,
                auth_detail: None,
                recommended: true,
                sort_order: 0,
                capabilities: InferenceCapabilities::coding_agent_default(),
                models: models
                    .into_iter()
                    .map(|model| {
                        let context_window = roder_api::catalog::lookup_model_for_provider(
                            self.backend.id(),
                            &model.id,
                        )
                        .map(|entry| entry.context_window);
                        ModelDescriptor {
                            id: model.id,
                            name: model.name,
                            context_window,
                            default_reasoning: Some(model.default_reasoning),
                            supported_reasoning: model
                                .reasoning_efforts
                                .into_iter()
                                .map(|effort| ReasoningEffortDescriptor {
                                    effort,
                                    description: String::new(),
                                })
                                .collect(),
                        }
                    })
                    .collect(),
            }],
        })?)
    }

    pub(super) async fn select_model(&self, method: &str, params: Value) -> anyhow::Result<Value> {
        let (provider, model, reasoning, thread_id) = if method == "providers/select" {
            let params: roder_protocol::ProviderSelectParams = serde_json::from_value(params)?;
            (
                params.provider,
                params.model,
                params.reasoning,
                params.thread_id,
            )
        } else {
            let params: roder_protocol::ModelSelectParams = serde_json::from_value(params)?;
            match params.selection {
                roder_protocol::ModelSelectChoice::Manual {
                    provider,
                    model,
                    reasoning,
                } => (provider, model, reasoning, params.thread_id),
                _ => anyhow::bail!("automatic model routing is unavailable for this backend"),
            }
        };
        if provider != self.backend.id() {
            anyhow::bail!("selected provider must be {}", self.backend.id());
        }
        let (models, current_model, _) = self.catalog().await?;
        let model = model.unwrap_or(current_model);
        let selected = models
            .iter()
            .find(|candidate| candidate.id == model)
            .ok_or_else(|| anyhow::anyhow!("unknown {} model {model}", self.backend.id()))?;
        let reasoning = reasoning.unwrap_or_else(|| selected.default_reasoning.clone());
        *self.selected_model.write().await = Some((model.clone(), reasoning.clone()));
        if let Some(thread_id) = thread_id {
            if let Some(handle) = self.threads.read().await.get(&thread_id).cloned() {
                handle.thread.write().await.model = model.clone();
            }
        }
        if method == "providers/select" {
            Ok(serde_json::to_value(ProviderSelectResult {
                provider,
                model,
                reasoning,
                model_profile: None,
                model_switch_summary: None,
            })?)
        } else {
            Ok(serde_json::to_value(ModelSelectResult {
                selection_mode: ModelSelectionMode::manual(
                    provider.clone(),
                    model.clone(),
                    Some(reasoning.clone()),
                ),
                provider,
                model,
                reasoning,
                model_profile: None,
                model_switch_summary: None,
            })?)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::backend::{BackendModel, BackendThreadSummary};
    use roder_api::extension::ExtensionRegistryBuilder;
    use roder_core::{RuntimeConfig, fake_provider::FakeInferenceEngine};
    use std::path::PathBuf;

    struct CatalogBackend;

    #[async_trait::async_trait]
    impl AgentBackend for CatalogBackend {
        fn id(&self) -> &'static str {
            "antigravity"
        }

        async fn connect(&self, _: BackendStart) -> anyhow::Result<Box<dyn AgentBackendSession>> {
            anyhow::bail!("catalog-only test backend")
        }

        async fn list_threads(
            &self,
            _: PathBuf,
            _: Option<usize>,
        ) -> anyhow::Result<Vec<BackendThreadSummary>> {
            Ok(vec![])
        }

        async fn list_models(&self, _: PathBuf) -> anyhow::Result<Vec<BackendModel>> {
            Ok(vec![BackendModel {
                id: "account-model".into(),
                name: "Account model".into(),
                is_default: true,
                default_reasoning: "high".into(),
                reasoning_efforts: vec!["high".into()],
            }])
        }
    }

    fn bridge() -> AgentBackendBridge {
        let mut builder = ExtensionRegistryBuilder::new();
        builder.inference_engine(Arc::new(FakeInferenceEngine));
        let runtime = Runtime::new(builder.build().unwrap(), RuntimeConfig::default()).unwrap();
        AgentBackendBridge::new(Arc::new(CatalogBackend), Arc::new(runtime))
    }

    #[test]
    fn backend_names_follow_the_selected_runtime() {
        assert_eq!(backend_name("codex"), "Codex");
        assert_eq!(backend_name("antigravity"), "Antigravity");
        assert_eq!(backend_name("custom"), "custom");
    }

    #[tokio::test]
    async fn backend_catalog_uses_selected_provider_and_account_models() {
        let bridge = bridge();
        let providers = bridge.providers().await.unwrap();
        assert_eq!(providers["active_provider"], "antigravity");
        assert_eq!(providers["providers"][0]["name"], "Antigravity");
        assert_eq!(
            providers["providers"][0]["models"][0]["id"],
            "account-model"
        );
        let models = bridge.models().await.unwrap();
        assert_eq!(models["models"][0]["modelProvider"], "antigravity");
        assert_eq!(models["models"][0]["id"], "account-model");
    }

    #[tokio::test]
    async fn unavailable_backend_model_names_the_selected_provider() {
        let error = bridge()
            .select_model(
                "providers/select",
                json!({"provider": "antigravity", "model": "unavailable"}),
            )
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "unknown antigravity model unavailable");
    }
}
