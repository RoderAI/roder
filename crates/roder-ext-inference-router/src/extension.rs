use std::sync::Arc;

use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use roder_api::inference_routing::{
    InferenceRouter, InferenceRoutingContext, InferenceRoutingDecision,
    InferenceRoutingOptionDescriptor,
};

use crate::config::LocalInferenceRouterConfig;

pub const LOCAL_INFERENCE_ROUTER_ID: &str = "local";

#[derive(Debug, Clone, Default)]
pub struct LocalInferenceRouterExtension {
    config: LocalInferenceRouterConfig,
}

impl LocalInferenceRouterExtension {
    pub fn new(config: LocalInferenceRouterConfig) -> Self {
        Self { config }
    }
}

impl RoderExtension for LocalInferenceRouterExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "roder-ext-inference-router".to_string(),
            name: "Local Inference Router".to_string(),
            version: semver::Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some("Local policy router for adaptive model selection".to_string()),
            provides: vec![ProvidedService::InferenceRouter(
                LOCAL_INFERENCE_ROUTER_ID.to_string(),
            )],
            required_capabilities: Vec::new(),
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.inference_router(Arc::new(LocalInferenceRouter::new(self.config.clone())));
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct LocalInferenceRouter {
    config: LocalInferenceRouterConfig,
}

impl LocalInferenceRouter {
    pub fn new(config: LocalInferenceRouterConfig) -> Self {
        Self { config }
    }
}

#[async_trait::async_trait]
impl InferenceRouter for LocalInferenceRouter {
    fn id(&self) -> String {
        LOCAL_INFERENCE_ROUTER_ID.to_string()
    }

    fn routing_options(&self) -> Vec<InferenceRoutingOptionDescriptor> {
        self.config.routing_options(LOCAL_INFERENCE_ROUTER_ID)
    }

    async fn route(
        &self,
        context: InferenceRoutingContext,
    ) -> anyhow::Result<InferenceRoutingDecision> {
        Ok(crate::policy::route(&self.config, context))
    }
}
