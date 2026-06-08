use roder_api::embeddings::{
    EmbeddingModelDescriptor, EmbeddingProvider, EmbeddingProviderDescriptor, EmbeddingRequest,
    EmbeddingResponse, EmbeddingVector,
};
use serde::{Deserialize, Serialize};

pub const DEFAULT_MODEL: &str = "text-embedding-3-large";
pub const DEFAULT_DIMENSIONS: usize = 3072;

#[derive(Clone)]
pub struct OpenAiEmbeddingProvider {
    api_key: Option<String>,
    base_url: String,
}

impl OpenAiEmbeddingProvider {
    pub fn new(api_key: Option<String>) -> Self {
        Self {
            api_key,
            base_url: "https://api.openai.com/v1".to_string(),
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }
}

#[async_trait::async_trait]
impl EmbeddingProvider for OpenAiEmbeddingProvider {
    fn descriptor(&self) -> EmbeddingProviderDescriptor {
        EmbeddingProviderDescriptor {
            id: "openai".to_string(),
            name: "OpenAI".to_string(),
            default_model: DEFAULT_MODEL.to_string(),
            models: vec![EmbeddingModelDescriptor {
                id: DEFAULT_MODEL.to_string(),
                dimensions: DEFAULT_DIMENSIONS,
                default: true,
            }],
        }
    }

    async fn embed(&self, request: EmbeddingRequest) -> anyhow::Result<EmbeddingResponse> {
        let api_key = self
            .api_key
            .clone()
            .or_else(|| std::env::var("OPENAI_API_KEY").ok())
            .filter(|key| !key.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("OPENAI_API_KEY is required for OpenAI embeddings"))?;
        if request.inputs.is_empty() {
            anyhow::bail!("embedding request must include at least one input");
        }

        let body = OpenAiEmbeddingRequest {
            model: request.model.clone(),
            input: request.inputs,
            dimensions: request.dimensions,
        };
        let response = reqwest::Client::new()
            .post(format!(
                "{}/embeddings",
                self.base_url.trim_end_matches('/')
            ))
            .bearer_auth(api_key)
            .json(&body)
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let message = response.text().await.unwrap_or_default();
            anyhow::bail!("OpenAI embeddings request failed with {status}: {message}");
        }
        let payload: OpenAiEmbeddingResponse = response.json().await?;
        let embeddings = payload
            .data
            .into_iter()
            .map(|item| EmbeddingVector {
                index: item.index,
                values: item.embedding,
            })
            .collect::<Vec<_>>();
        if embeddings
            .iter()
            .any(|embedding| embedding.values.is_empty())
        {
            anyhow::bail!("OpenAI returned an empty embedding vector");
        }
        Ok(EmbeddingResponse {
            provider_id: "openai".to_string(),
            model: payload.model.unwrap_or(request.model),
            embeddings,
        })
    }
}

#[derive(Debug, Serialize)]
struct OpenAiEmbeddingRequest {
    model: String,
    input: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dimensions: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct OpenAiEmbeddingResponse {
    #[serde(default)]
    model: Option<String>,
    data: Vec<OpenAiEmbeddingItem>,
}

#[derive(Debug, Deserialize)]
struct OpenAiEmbeddingItem {
    index: usize,
    embedding: Vec<f32>,
}

#[cfg(test)]
mod tests {
    use roder_api::embeddings::EmbeddingInputType;

    use super::*;

    #[test]
    fn registers_openai_embedding_descriptor() {
        let descriptor = OpenAiEmbeddingProvider::new(None).descriptor();
        assert_eq!(descriptor.id, "openai");
        assert_eq!(descriptor.default_model, DEFAULT_MODEL);
        assert_eq!(descriptor.models[0].dimensions, DEFAULT_DIMENSIONS);
    }

    #[tokio::test]
    async fn missing_key_returns_concise_error() {
        let err = OpenAiEmbeddingProvider::new(Some(String::new()))
            .with_base_url("http://127.0.0.1:1")
            .embed(EmbeddingRequest {
                model: DEFAULT_MODEL.to_string(),
                inputs: vec!["hello".to_string()],
                input_type: EmbeddingInputType::Document,
                dimensions: None,
            })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("OPENAI_API_KEY"));
    }
}
