use base64::Engine;
use roder_api::embeddings::{
    EmbeddingModelDescriptor, EmbeddingProvider, EmbeddingProviderDescriptor, EmbeddingRequest,
    EmbeddingResponse, EmbeddingVector,
};
use serde::{Deserialize, Serialize};

pub const ZEROENTROPY_EMBEDDING_PROVIDER_ID: &str = "zeroentropy";
pub const DEFAULT_MODEL: &str = "zembed-1";
pub const DEFAULT_DIMENSIONS: usize = 2560;
pub const DEFAULT_ENDPOINT: &str = "https://api.zeroentropy.dev/v1";

const KEY_ENV_VARS: &[&str] = &["RODER_ZEROENTROPY_API_KEY", "ZEROENTROPY_API_KEY"];
const SUPPORTED_DIMENSIONS: &[usize] = &[2560, 1280, 640, 320, 160, 80, 40];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ZeroEntropyEncodingFormat {
    Float,
    #[default]
    Base64,
}

impl ZeroEntropyEncodingFormat {
    fn as_str(self) -> &'static str {
        match self {
            Self::Float => "float",
            Self::Base64 => "base64",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZeroEntropyLatency {
    Fast,
    Slow,
}

impl ZeroEntropyLatency {
    fn as_str(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Slow => "slow",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ZeroEntropyEmbeddingsConfig {
    pub api_key: Option<String>,
    pub endpoint: String,
    pub encoding_format: ZeroEntropyEncodingFormat,
    pub latency: Option<ZeroEntropyLatency>,
}

impl Default for ZeroEntropyEmbeddingsConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            endpoint: DEFAULT_ENDPOINT.to_string(),
            encoding_format: ZeroEntropyEncodingFormat::default(),
            latency: None,
        }
    }
}

impl ZeroEntropyEmbeddingsConfig {
    pub fn from_env() -> Self {
        Self {
            api_key: zeroentropy_embedding_api_key_from_env(),
            endpoint: std::env::var("RODER_ZEROENTROPY_EMBEDDINGS_ENDPOINT")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_ENDPOINT.to_string()),
            ..Self::default()
        }
    }
}

#[derive(Clone)]
pub struct ZeroEntropyEmbeddingProvider {
    config: ZeroEntropyEmbeddingsConfig,
    client: reqwest::Client,
}

impl ZeroEntropyEmbeddingProvider {
    pub fn new(config: ZeroEntropyEmbeddingsConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }

    pub fn with_api_key(api_key: impl Into<String>) -> Self {
        Self::new(ZeroEntropyEmbeddingsConfig {
            api_key: Some(api_key.into()),
            ..ZeroEntropyEmbeddingsConfig::default()
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
            .or_else(zeroentropy_embedding_api_key_from_env)
            .filter(|key| !key.trim().is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "ZeroEntropy embeddings require one of: {}",
                    KEY_ENV_VARS.join(", ")
                )
            })
    }
}

#[async_trait::async_trait]
impl EmbeddingProvider for ZeroEntropyEmbeddingProvider {
    fn descriptor(&self) -> EmbeddingProviderDescriptor {
        EmbeddingProviderDescriptor {
            id: ZEROENTROPY_EMBEDDING_PROVIDER_ID.to_string(),
            name: "ZeroEntropy".to_string(),
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
        let model = if request.model.trim().is_empty() {
            DEFAULT_MODEL.to_string()
        } else {
            request.model.clone()
        };
        if model.trim() != DEFAULT_MODEL {
            anyhow::bail!("ZeroEntropy embeddings support only {DEFAULT_MODEL}");
        }
        if let Some(dimensions) = request.dimensions
            && !SUPPORTED_DIMENSIONS.contains(&dimensions)
        {
            anyhow::bail!(
                "ZeroEntropy zembed-1 dimensions must be one of: {}",
                SUPPORTED_DIMENSIONS
                    .iter()
                    .map(|dim| dim.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }

        let url = format!(
            "{}/models/embed",
            self.config.endpoint.trim_end_matches('/')
        );
        let body = ZeroEntropyEmbeddingRequest {
            model: DEFAULT_MODEL,
            input_type: request.input_type.as_zeroentropy_str(),
            input: request.inputs,
            dimensions: request.dimensions,
            encoding_format: self.config.encoding_format.as_str(),
            latency: self.config.latency.map(ZeroEntropyLatency::as_str),
        };
        let response = self
            .client
            .post(url)
            .bearer_auth(api_key)
            .json(&body)
            .send()
            .await?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            anyhow::bail!("ZeroEntropy embeddings request failed with {status}: {text}");
        }
        let payload: ZeroEntropyEmbeddingResponse = serde_json::from_str(&text)?;
        if payload.results.len() != body.input.len() {
            anyhow::bail!(
                "ZeroEntropy returned {} embeddings for {} inputs",
                payload.results.len(),
                body.input.len()
            );
        }
        let mut embeddings = Vec::with_capacity(payload.results.len());
        for (index, result) in payload.results.into_iter().enumerate() {
            let values = result.embedding.into_values()?;
            if values.is_empty() {
                anyhow::bail!("ZeroEntropy returned an empty embedding vector");
            }
            embeddings.push(EmbeddingVector { index, values });
        }
        Ok(EmbeddingResponse {
            provider_id: ZEROENTROPY_EMBEDDING_PROVIDER_ID.to_string(),
            model,
            embeddings,
        })
    }
}

trait ZeroEntropyInputType {
    fn as_zeroentropy_str(&self) -> &'static str;
}

impl ZeroEntropyInputType for roder_api::embeddings::EmbeddingInputType {
    fn as_zeroentropy_str(&self) -> &'static str {
        match self {
            Self::Query => "query",
            Self::Document => "document",
        }
    }
}

#[derive(Debug, Serialize)]
struct ZeroEntropyEmbeddingRequest<'a> {
    model: &'a str,
    input_type: &'a str,
    input: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dimensions: Option<usize>,
    encoding_format: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    latency: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
struct ZeroEntropyEmbeddingResponse {
    results: Vec<ZeroEntropyEmbeddingResult>,
}

#[derive(Debug, Deserialize)]
struct ZeroEntropyEmbeddingResult {
    embedding: ZeroEntropyEmbeddingValue,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ZeroEntropyEmbeddingValue {
    Float(Vec<f32>),
    Base64(String),
}

impl ZeroEntropyEmbeddingValue {
    fn into_values(self) -> anyhow::Result<Vec<f32>> {
        match self {
            Self::Float(values) => Ok(values),
            Self::Base64(encoded) => decode_base64_embedding(&encoded),
        }
    }
}

fn decode_base64_embedding(encoded: &str) -> anyhow::Result<Vec<f32>> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|err| anyhow::anyhow!("invalid ZeroEntropy base64 embedding: {err}"))?;
    if bytes.len() % std::mem::size_of::<f32>() != 0 {
        anyhow::bail!(
            "invalid ZeroEntropy base64 embedding length {}; expected f32 byte multiple",
            bytes.len()
        );
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect())
}

fn zeroentropy_embedding_api_key_from_env() -> Option<String> {
    KEY_ENV_VARS.iter().find_map(|name| {
        std::env::var(name)
            .ok()
            .filter(|value| !value.trim().is_empty())
    })
}
