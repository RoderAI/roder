use roder_api::embeddings::{
    EmbeddingModelDescriptor, EmbeddingProvider, EmbeddingProviderDescriptor, EmbeddingRequest,
    EmbeddingResponse, EmbeddingVector,
};
use serde::{Deserialize, Serialize};

pub const GOOGLE_EMBEDDING_PROVIDER_ID: &str = "google";
pub const DEFAULT_MODEL: &str = "gemini-embedding-2";
pub const DEFAULT_DIMENSIONS: usize = 3072;
pub const DEFAULT_ENDPOINT: &str = "https://generativelanguage.googleapis.com/v1beta";

const KEY_ENV_VARS: &[&str] = &[
    "RODER_GOOGLE_EMBEDDINGS_API_KEY",
    "GEMINI_API_TOKEN",
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "GOOGLE_GENAI_API_KEY",
    "GOOGLE_AI_API_KEY",
];

#[derive(Debug, Clone)]
pub struct GoogleEmbeddingsConfig {
    pub api_key: Option<String>,
    pub endpoint: String,
}

impl Default for GoogleEmbeddingsConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            endpoint: DEFAULT_ENDPOINT.to_string(),
        }
    }
}

impl GoogleEmbeddingsConfig {
    pub fn from_env() -> Self {
        Self {
            api_key: google_embedding_api_key_from_env(),
            endpoint: std::env::var("RODER_GOOGLE_EMBEDDINGS_ENDPOINT")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_ENDPOINT.to_string()),
        }
    }
}

#[derive(Clone)]
pub struct GoogleEmbeddingProvider {
    config: GoogleEmbeddingsConfig,
    client: reqwest::Client,
}

impl GoogleEmbeddingProvider {
    pub fn new(config: GoogleEmbeddingsConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }

    pub fn with_api_key(api_key: impl Into<String>) -> Self {
        Self::new(GoogleEmbeddingsConfig {
            api_key: Some(api_key.into()),
            ..GoogleEmbeddingsConfig::default()
        })
    }

    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.config.endpoint = endpoint.into();
        self
    }

    fn api_key(&self) -> anyhow::Result<String> {
        self.config
            .api_key
            .clone()
            .or_else(google_embedding_api_key_from_env)
            .filter(|key| !key.trim().is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Google embeddings require one of: {}",
                    KEY_ENV_VARS.join(", ")
                )
            })
    }

    fn model_path(model: &str) -> String {
        let model = model.trim();
        if model.starts_with("models/") {
            model.to_string()
        } else {
            format!("models/{model}")
        }
    }

    fn model_id(model: &str) -> &str {
        model.trim().strip_prefix("models/").unwrap_or(model.trim())
    }
}

#[async_trait::async_trait]
impl EmbeddingProvider for GoogleEmbeddingProvider {
    fn descriptor(&self) -> EmbeddingProviderDescriptor {
        EmbeddingProviderDescriptor {
            id: GOOGLE_EMBEDDING_PROVIDER_ID.to_string(),
            name: "Google Gemini Embeddings".to_string(),
            default_model: DEFAULT_MODEL.to_string(),
            models: vec![EmbeddingModelDescriptor {
                id: DEFAULT_MODEL.to_string(),
                dimensions: DEFAULT_DIMENSIONS,
                default: true,
            }],
        }
    }

    async fn embed(&self, request: EmbeddingRequest) -> anyhow::Result<EmbeddingResponse> {
        let api_key = self.api_key()?;
        if request.inputs.is_empty() {
            anyhow::bail!("embedding request must include at least one input");
        }
        if let Some(dimensions) = request.dimensions {
            if dimensions == 0 || dimensions > DEFAULT_DIMENSIONS {
                anyhow::bail!(
                    "Google embeddings dimensions must be between 1 and {DEFAULT_DIMENSIONS}"
                );
            }
        }

        let model = if request.model.trim().is_empty() {
            DEFAULT_MODEL.to_string()
        } else {
            request.model.clone()
        };
        if Self::model_id(&model) != DEFAULT_MODEL {
            anyhow::bail!("Google embeddings support only {DEFAULT_MODEL}");
        }
        let mut embeddings = Vec::with_capacity(request.inputs.len());
        for (index, input) in request.inputs.into_iter().enumerate() {
            let values = self
                .embed_one(&api_key, &model, input, request.dimensions)
                .await?;
            embeddings.push(EmbeddingVector { index, values });
        }
        Ok(EmbeddingResponse {
            provider_id: GOOGLE_EMBEDDING_PROVIDER_ID.to_string(),
            model: Self::model_id(&model).to_string(),
            embeddings,
        })
    }
}

impl GoogleEmbeddingProvider {
    async fn embed_one(
        &self,
        api_key: &str,
        model: &str,
        input: String,
        dimensions: Option<usize>,
    ) -> anyhow::Result<Vec<f32>> {
        let mut url = reqwest::Url::parse(&format!(
            "{}/models/{}:embedContent",
            self.config.endpoint.trim_end_matches('/'),
            Self::model_id(model)
        ))?;
        url.query_pairs_mut().append_pair("key", api_key);
        let body = GoogleEmbeddingRequest {
            model: Self::model_path(model),
            content: GoogleContent {
                parts: vec![GooglePart { text: input }],
            },
            output_dimensionality: dimensions,
        };
        let response = self.client.post(url).json(&body).send().await?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            anyhow::bail!("Google embeddings request failed with {status}: {text}");
        }
        let payload: GoogleEmbeddingResponse = serde_json::from_str(&text)?;
        if payload.embedding.values.is_empty() {
            anyhow::bail!("Google returned an empty embedding vector");
        }
        Ok(payload.embedding.values)
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GoogleEmbeddingRequest {
    model: String,
    content: GoogleContent,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_dimensionality: Option<usize>,
}

#[derive(Debug, Serialize)]
struct GoogleContent {
    parts: Vec<GooglePart>,
}

#[derive(Debug, Serialize)]
struct GooglePart {
    text: String,
}

#[derive(Debug, Deserialize)]
struct GoogleEmbeddingResponse {
    embedding: GoogleEmbedding,
}

#[derive(Debug, Deserialize)]
struct GoogleEmbedding {
    values: Vec<f32>,
}

pub fn query_input(query: &str) -> String {
    format!("task: search result | query: {}", query.trim())
}

pub fn document_input(title: Option<&str>, text: &str) -> String {
    match title.map(str::trim).filter(|title| !title.is_empty()) {
        Some(title) => format!("title: {title} | text: {}", text.trim()),
        None => format!("text: {}", text.trim()),
    }
}

fn google_embedding_api_key_from_env() -> Option<String> {
    KEY_ENV_VARS.iter().find_map(|name| {
        std::env::var(name)
            .ok()
            .filter(|value| !value.trim().is_empty())
    })
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::sync::{Arc, Mutex};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    #[test]
    fn registers_google_embedding_descriptor() {
        let descriptor =
            GoogleEmbeddingProvider::new(GoogleEmbeddingsConfig::default()).descriptor();
        assert_eq!(descriptor.id, "google");
        assert_eq!(descriptor.default_model, DEFAULT_MODEL);
        assert_eq!(descriptor.models[0].dimensions, DEFAULT_DIMENSIONS);
    }

    #[test]
    fn formats_retrieval_inputs() {
        assert_eq!(
            query_input("  outage policy "),
            "task: search result | query: outage policy"
        );
        assert_eq!(
            document_input(Some("Runbook"), "  restart service "),
            "title: Runbook | text: restart service"
        );
        assert_eq!(
            document_input(None, "  restart service "),
            "text: restart service"
        );
    }

    #[tokio::test]
    async fn missing_key_returns_concise_error() {
        let err = GoogleEmbeddingProvider::new(GoogleEmbeddingsConfig {
            api_key: Some(String::new()),
            ..GoogleEmbeddingsConfig::default()
        })
        .with_endpoint("http://127.0.0.1:1")
        .embed(EmbeddingRequest {
            model: DEFAULT_MODEL.to_string(),
            inputs: vec!["hello".to_string()],
            dimensions: None,
        })
        .await
        .unwrap_err();
        assert!(err.to_string().contains("Google embeddings require"));
    }

    #[tokio::test]
    async fn maps_request_and_response() {
        let server = TestServer::start(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"embedding\":{\"values\":[0.1,0.2,0.3]}}\n",
            1,
        )
        .await;
        let provider =
            GoogleEmbeddingProvider::with_api_key("test-key").with_endpoint(server.endpoint());
        let response = provider
            .embed(EmbeddingRequest {
                model: DEFAULT_MODEL.to_string(),
                inputs: vec!["hello".to_string()],
                dimensions: Some(3),
            })
            .await
            .unwrap();

        assert_eq!(response.provider_id, "google");
        assert_eq!(response.model, DEFAULT_MODEL);
        assert_eq!(response.embeddings[0].index, 0);
        assert_eq!(response.embeddings[0].values, vec![0.1, 0.2, 0.3]);
        let request = server.requests().join("\n");
        assert!(request.contains("POST /models/gemini-embedding-2:embedContent?key=test-key"));
        assert!(request.contains("\"model\":\"models/gemini-embedding-2\""));
        assert!(request.contains("\"text\":\"hello\""));
        assert!(request.contains("\"outputDimensionality\":3"));
    }

    #[tokio::test]
    async fn unsupported_model_returns_concise_error() {
        let provider = GoogleEmbeddingProvider::with_api_key("test-key");
        let err = provider
            .embed(EmbeddingRequest {
                model: "text-embedding-004".to_string(),
                inputs: vec!["hello".to_string()],
                dimensions: None,
            })
            .await
            .unwrap_err()
            .to_string();

        assert!(err.contains(DEFAULT_MODEL));
    }

    #[tokio::test]
    async fn preserves_input_order_with_sequential_calls() {
        let server = TestServer::start(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"embedding\":{\"values\":[0.4]}}\n",
            2,
        )
        .await;
        let provider =
            GoogleEmbeddingProvider::with_api_key("test-key").with_endpoint(server.endpoint());
        let response = provider
            .embed(EmbeddingRequest {
                model: DEFAULT_MODEL.to_string(),
                inputs: vec!["first".to_string(), "second".to_string()],
                dimensions: Some(1),
            })
            .await
            .unwrap();

        assert_eq!(
            response
                .embeddings
                .iter()
                .map(|embedding| embedding.index)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
        let request = server.requests().join("\n");
        assert!(request.contains("\"text\":\"first\""));
        assert!(request.contains("\"text\":\"second\""));
    }

    #[tokio::test]
    async fn non_success_status_is_reported() {
        let server = TestServer::start(
            "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\n\r\n{\"error\":{\"message\":\"bad model\"}}\n",
            1,
        )
        .await;
        let provider =
            GoogleEmbeddingProvider::with_api_key("test-key").with_endpoint(server.endpoint());
        let err = provider
            .embed(EmbeddingRequest {
                model: DEFAULT_MODEL.to_string(),
                inputs: vec!["hello".to_string()],
                dimensions: None,
            })
            .await
            .unwrap_err();

        assert!(err.to_string().contains("400 Bad Request"));
        assert!(err.to_string().contains("bad model"));
    }

    struct TestServer {
        addr: SocketAddr,
        requests: Arc<Mutex<Vec<String>>>,
    }

    impl TestServer {
        async fn start(response: &'static str, expected_requests: usize) -> Self {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let requests = Arc::new(Mutex::new(Vec::new()));
            let captured = requests.clone();
            tokio::spawn(async move {
                for _ in 0..expected_requests {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    let mut buffer = [0_u8; 8192];
                    let read = stream.read(&mut buffer).await.unwrap();
                    captured
                        .lock()
                        .unwrap()
                        .push(String::from_utf8_lossy(&buffer[..read]).to_string());
                    stream.write_all(response.as_bytes()).await.unwrap();
                }
            });
            Self { addr, requests }
        }

        fn endpoint(&self) -> String {
            format!("http://{}", self.addr)
        }

        fn requests(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }
    }
}
