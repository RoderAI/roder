use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::extension::EmbeddingProviderId;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingModelDescriptor {
    pub id: String,
    pub dimensions: usize,
    #[serde(default)]
    pub default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingProviderDescriptor {
    pub id: EmbeddingProviderId,
    pub name: String,
    pub default_model: String,
    pub models: Vec<EmbeddingModelDescriptor>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EmbeddingInputType {
    Query,
    #[default]
    Document,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingRequest {
    pub model: String,
    pub inputs: Vec<String>,
    #[serde(default)]
    pub input_type: EmbeddingInputType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingVector {
    pub index: usize,
    pub values: Vec<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingResponse {
    pub provider_id: EmbeddingProviderId,
    pub model: String,
    pub embeddings: Vec<EmbeddingVector>,
}

#[async_trait::async_trait]
pub trait EmbeddingProvider: Send + Sync + 'static {
    fn descriptor(&self) -> EmbeddingProviderDescriptor;

    async fn embed(&self, request: EmbeddingRequest) -> anyhow::Result<EmbeddingResponse>;
}

#[derive(Clone)]
pub struct EmbeddingProviderFactory {
    provider: Arc<dyn EmbeddingProvider>,
}

impl EmbeddingProviderFactory {
    pub fn new(provider: Arc<dyn EmbeddingProvider>) -> Self {
        Self { provider }
    }

    pub fn id(&self) -> EmbeddingProviderId {
        self.provider.descriptor().id
    }

    pub fn create(&self) -> Arc<dyn EmbeddingProvider> {
        self.provider.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedding_request_serializes_query_intent() {
        let request = EmbeddingRequest {
            model: "zembed-1".to_string(),
            inputs: vec!["what changed?".to_string()],
            input_type: EmbeddingInputType::Query,
            dimensions: Some(2560),
        };

        let json = serde_json::to_value(request).unwrap();

        assert_eq!(json["inputType"], "query");
        assert_eq!(json["dimensions"], 2560);
    }

    #[test]
    fn embedding_request_defaults_to_document_intent() {
        let request: EmbeddingRequest = serde_json::from_value(serde_json::json!({
            "model": "zembed-1",
            "inputs": ["stored memory"]
        }))
        .unwrap();

        assert_eq!(request.input_type, EmbeddingInputType::Document);
    }
}
