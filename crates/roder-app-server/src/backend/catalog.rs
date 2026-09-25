use super::*;
use roder_api::inference::{
    InferenceCapabilities, ModelDescriptor, ProviderAuthType, ReasoningEffortDescriptor,
};
use roder_api::inference_routing::ModelSelectionMode;
use roder_protocol::{
    Model, ModelListResult, ModelSelectResult, ProviderDescriptor, ProviderSelectResult,
    ProvidersListResult,
};

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
                name: "Codex".into(),
                description: Some("Codex app-server agent backend".into()),
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
            .ok_or_else(|| anyhow::anyhow!("unknown Codex model {model}"))?;
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
