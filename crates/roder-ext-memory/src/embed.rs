use std::sync::Arc;

use roder_api::embeddings::{EmbeddingProvider, EmbeddingRequest};

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

    pub async fn embed(&self, text: &str, model: Option<&str>) -> MemoryEmbedding {
        let model = model
            .map(str::trim)
            .filter(|model| !model.is_empty())
            .unwrap_or(self.model.as_str());
        if let Some(provider) = &self.provider {
            let request = EmbeddingRequest {
                model: model.to_string(),
                inputs: vec![text.to_string()],
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
