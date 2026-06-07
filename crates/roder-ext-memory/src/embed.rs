use std::sync::Arc;

use roder_api::embeddings::{EmbeddingInputType, EmbeddingProvider, EmbeddingRequest};

use crate::vector;

pub const FALLBACK_PROVIDER: &str = "fake";
pub const FALLBACK_MODEL: &str = "fake-vector-32";

#[derive(Debug, Clone)]
pub struct MemoryEmbedding {
    pub provider_id: String,
    pub model: String,
    pub values: Vec<f32>,
}

#[derive(Clone)]
pub struct MemoryEmbedder {
    provider: Option<Arc<dyn EmbeddingProvider>>,
    provider_id: String,
    model: String,
}

impl MemoryEmbedder {
    pub fn new(provider: Option<Arc<dyn EmbeddingProvider>>) -> Self {
        match &provider {
            Some(provider) => {
                let descriptor = provider.descriptor();
                Self {
                    provider: Some(provider.clone()),
                    provider_id: descriptor.id,
                    model: descriptor.default_model,
                }
            }
            None => Self {
                provider: None,
                provider_id: FALLBACK_PROVIDER.to_string(),
                model: FALLBACK_MODEL.to_string(),
            },
        }
    }

    pub fn provider_id(&self) -> &str {
        &self.provider_id
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn can_embed_provider(&self, provider_id: &str) -> bool {
        provider_id == self.provider_id || provider_id == FALLBACK_PROVIDER
    }

    pub async fn embed_document(&self, text: &str, model: Option<&str>) -> MemoryEmbedding {
        self.embed_with_type(text, model, EmbeddingInputType::Document)
            .await
    }

    pub async fn embed_query(&self, text: &str, model: Option<&str>) -> MemoryEmbedding {
        self.embed_with_type(text, model, EmbeddingInputType::Query)
            .await
    }

    async fn embed_with_type(
        &self,
        text: &str,
        model: Option<&str>,
        input_type: EmbeddingInputType,
    ) -> MemoryEmbedding {
        let model = model
            .map(str::trim)
            .filter(|model| !model.is_empty())
            .unwrap_or(self.model.as_str());
        if let Some(provider) = &self.provider {
            let request = EmbeddingRequest {
                model: model.to_string(),
                inputs: vec![text.to_string()],
                input_type,
                dimensions: None,
            };
            match provider.embed(request).await {
                Ok(response) => {
                    if let Some(vector) = response.embeddings.into_iter().next()
                        && !vector.values.is_empty()
                    {
                        return MemoryEmbedding {
                            provider_id: response.provider_id,
                            model: response.model,
                            values: vector.values,
                        };
                    }
                    eprintln!(
                        "memory: embedding provider '{}' returned no vector; using deterministic fallback",
                        self.provider_id
                    );
                }
                Err(err) => {
                    eprintln!(
                        "memory: embedding provider '{}' unavailable ({err}); using deterministic fallback",
                        self.provider_id
                    );
                }
            }
        }
        Self::fallback_embedding(text)
    }

    pub fn fallback_embedding(text: &str) -> MemoryEmbedding {
        MemoryEmbedding {
            provider_id: FALLBACK_PROVIDER.to_string(),
            model: FALLBACK_MODEL.to_string(),
            values: vector::fake_embedding(text),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::embeddings::{
        EmbeddingModelDescriptor, EmbeddingProviderDescriptor, EmbeddingResponse, EmbeddingVector,
    };
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct RecordingProvider {
        seen: Arc<Mutex<Vec<EmbeddingInputType>>>,
    }

    #[async_trait::async_trait]
    impl EmbeddingProvider for RecordingProvider {
        fn descriptor(&self) -> EmbeddingProviderDescriptor {
            EmbeddingProviderDescriptor {
                id: "recording".to_string(),
                name: "Recording".to_string(),
                default_model: "recording-model".to_string(),
                models: vec![EmbeddingModelDescriptor {
                    id: "recording-model".to_string(),
                    dimensions: 1,
                    default: true,
                }],
            }
        }

        async fn embed(&self, request: EmbeddingRequest) -> anyhow::Result<EmbeddingResponse> {
            self.seen.lock().unwrap().push(request.input_type);
            Ok(EmbeddingResponse {
                provider_id: "recording".to_string(),
                model: request.model,
                embeddings: vec![EmbeddingVector {
                    index: 0,
                    values: vec![1.0],
                }],
            })
        }
    }

    #[tokio::test]
    async fn memory_embedder_sends_document_and_query_intents() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let embedder =
            MemoryEmbedder::new(Some(Arc::new(RecordingProvider { seen: seen.clone() })));

        embedder.embed_document("stored memory", None).await;
        embedder.embed_query("find memory", None).await;

        assert_eq!(
            *seen.lock().unwrap(),
            vec![EmbeddingInputType::Document, EmbeddingInputType::Query]
        );
    }
}
