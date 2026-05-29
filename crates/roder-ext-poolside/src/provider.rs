use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use roder_api::catalog::{PROVIDER_POOLSIDE, models_for_provider};
use roder_api::extension::InferenceEngineId;
use roder_api::inference::{
    AgentInferenceRequest, InferenceCapabilities, InferenceEngine, InferenceEventStream,
    InferenceProviderContext, InferenceProviderMetadata, InferenceTurnContext, ModelDescriptor,
    ProviderAuthType,
};
use serde::{Deserialize, Serialize};

use crate::chat::stream_chat_completions;

const DEFAULT_BASE_URL: &str = "https://inference.poolside.ai/v1";
const DEFAULT_MODELS_CACHE_TTL: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Debug, Clone, Default)]
pub struct PoolsideConfig {
    pub api_key: Option<String>,
    pub base_url: Option<String>,
}

pub struct PoolsideInferenceEngine {
    config: PoolsideConfig,
    refresh_in_flight: Arc<AtomicBool>,
}

impl PoolsideInferenceEngine {
    pub fn new(config: PoolsideConfig) -> Self {
        Self {
            config,
            refresh_in_flight: Arc::new(AtomicBool::new(false)),
        }
    }

    fn base_url(&self) -> String {
        nonempty(self.config.base_url.clone())
            .or_else(config_base_url)
            .or_else(|| env_nonempty("RODER_POOLSIDE_BASE_URL"))
            .or_else(|| env_nonempty("POOLSIDE_BASE_URL"))
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
            .trim_end_matches('/')
            .to_string()
    }

    fn api_key(&self) -> Option<String> {
        nonempty(self.config.api_key.clone())
            .or_else(|| env_nonempty("POOLSIDE_API_KEY"))
            .or_else(|| env_nonempty("RODER_POOLSIDE_API_KEY"))
            .or_else(config_api_key)
    }

    fn schedule_model_refresh(&self, base_url: String, api_key: Option<String>) {
        if self
            .refresh_in_flight
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let refresh_in_flight = Arc::clone(&self.refresh_in_flight);
        tokio::spawn(async move {
            if let Ok(models) = discover_models(base_url.clone(), api_key).await {
                let _ = save_cached_models(&base_url, &models);
            }
            refresh_in_flight.store(false, Ordering::Release);
        });
    }
}

#[async_trait::async_trait]
impl InferenceEngine for PoolsideInferenceEngine {
    fn id(&self) -> InferenceEngineId {
        PROVIDER_POOLSIDE.to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities {
            streaming: true,
            tool_calls: true,
            parallel_tool_calls: true,
            reasoning_summaries: false,
            structured_output: true,
            image_input: false,
            prompt_cache: false,
            provider_metadata: true,
        }
    }

    fn metadata(&self) -> InferenceProviderMetadata {
        InferenceProviderMetadata {
            name: "Poolside".to_string(),
            description: Some("Poolside Laguna API key provider".to_string()),
            auth_type: ProviderAuthType::ApiKey,
            auth_label: Some("POOLSIDE_API_KEY".to_string()),
            auth_configured: Some(self.api_key().is_some()),
            recommended: true,
            sort_order: 17,
        }
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        let base_url = self.base_url();
        let cached = cached_models(&base_url).ok();
        let should_refresh = force_refresh_requested()
            || cached
                .as_ref()
                .map(|entry| entry.is_stale(cache_ttl()))
                .unwrap_or(true);

        if should_refresh {
            self.schedule_model_refresh(base_url, self.api_key());
        }

        if let Some(entry) = cached
            && !entry.models.is_empty()
        {
            return Ok(entry.models);
        }

        Ok(models_for_provider(PROVIDER_POOLSIDE, false))
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let Some(api_key) = self.api_key() else {
            anyhow::bail!(
                "Poolside API key is missing; set POOLSIDE_API_KEY or configure it from the provider menu"
            )
        };
        stream_chat_completions(&self.base_url(), &api_key, request).await
    }
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    #[serde(default)]
    data: Vec<ModelEntry>,
}

#[derive(Debug, Deserialize)]
struct ModelEntry {
    id: String,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ModelsCacheFile {
    #[serde(default)]
    providers: BTreeMap<String, CachedProviderModels>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedProviderModels {
    fetched_at: u64,
    base_url: String,
    models: Vec<ModelDescriptor>,
}

impl CachedProviderModels {
    fn is_stale(&self, ttl: Duration) -> bool {
        ttl.is_zero()
            || now_unix_secs()
                .saturating_sub(self.fetched_at)
                .ge(&ttl.as_secs())
    }
}

async fn discover_models(
    base_url: String,
    api_key: Option<String>,
) -> anyhow::Result<Vec<ModelDescriptor>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()?;
    let mut request = client.get(format!("{}/models", base_url.trim_end_matches('/')));
    if let Some(api_key) = api_key {
        request = request.bearer_auth(api_key);
    }
    let response = request.send().await?;
    if !response.status().is_success() {
        anyhow::bail!("Poolside model discovery failed: {}", response.status());
    }
    let body: ModelsResponse = response.json().await?;
    let models = body
        .data
        .into_iter()
        .filter_map(|model| {
            let id = model.id.trim();
            (!id.is_empty()).then(|| ModelDescriptor {
                id: id.to_string(),
                name: model.name.unwrap_or_else(|| id.to_string()),
                context_window: None,
                default_reasoning: None,
                supported_reasoning: Vec::new(),
            })
        })
        .collect::<Vec<_>>();
    if models.is_empty() {
        anyhow::bail!("Poolside model discovery returned no models");
    }
    Ok(models)
}

fn config_api_key() -> Option<String> {
    roder_config::load_config().ok().and_then(|config| {
        config
            .providers
            .get(PROVIDER_POOLSIDE)
            .and_then(|provider| {
                nonempty(provider.api_key.clone())
                    .or_else(|| provider.api_key_env.as_deref().and_then(env_nonempty))
            })
    })
}

fn config_base_url() -> Option<String> {
    roder_config::load_config().ok().and_then(|config| {
        config
            .providers
            .get(PROVIDER_POOLSIDE)
            .and_then(|provider| nonempty(provider.base_url.clone()))
    })
}

fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .and_then(|value| nonempty(Some(value)))
}

fn nonempty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn cached_models(base_url: &str) -> anyhow::Result<CachedProviderModels> {
    let cache: ModelsCacheFile = serde_json::from_str(&fs::read_to_string(cache_path())?)?;
    let entry = cache
        .providers
        .get(PROVIDER_POOLSIDE)
        .filter(|entry| entry.base_url.trim_end_matches('/') == base_url.trim_end_matches('/'))
        .cloned();
    entry.ok_or_else(|| anyhow::anyhow!("no cached models for poolside"))
}

fn save_cached_models(base_url: &str, models: &[ModelDescriptor]) -> anyhow::Result<()> {
    let path = cache_path();
    let mut cache = fs::read_to_string(&path)
        .ok()
        .and_then(|body| serde_json::from_str::<ModelsCacheFile>(&body).ok())
        .unwrap_or_default();
    cache.providers.insert(
        PROVIDER_POOLSIDE.to_string(),
        CachedProviderModels {
            fetched_at: now_unix_secs(),
            base_url: base_url.trim_end_matches('/').to_string(),
            models: models.to_vec(),
        },
    );
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(&cache)?)?;
    Ok(())
}

fn cache_path() -> PathBuf {
    if let Some(path) = env_nonempty("RODER_MODELS_CACHE_PATH") {
        return PathBuf::from(path);
    }
    roder_data_dir().join("models-cache.json")
}

fn roder_data_dir() -> PathBuf {
    std::env::var_os("RODER_DATA_DIR")
        .or_else(|| std::env::var_os("RODER_CONFIG_DIR"))
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".roder")
        })
}

fn cache_ttl() -> Duration {
    env_nonempty("RODER_MODELS_CACHE_TTL_SECONDS")
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_MODELS_CACHE_TTL)
}

fn force_refresh_requested() -> bool {
    env_nonempty("RODER_MODELS_REFRESH")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use roder_api::inference::{
        InferenceEvent, InstructionBundle, ModelSelection, OutputConfig, ReasoningConfig,
        RuntimeHints,
    };
    use roder_api::tools::{ToolChoice, ToolSpec};
    use roder_api::transcript::{TranscriptItem, UserMessage};
    use serde_json::{Value, json};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn poolside_metadata_uses_api_key_auth() {
        let engine = PoolsideInferenceEngine::new(PoolsideConfig::default());
        assert_eq!(engine.id(), PROVIDER_POOLSIDE);
        let metadata = engine.metadata();
        assert_eq!(metadata.name, "Poolside");
        assert_eq!(metadata.auth_type, ProviderAuthType::ApiKey);
        assert_eq!(metadata.auth_label.as_deref(), Some("POOLSIDE_API_KEY"));
    }

    #[tokio::test]
    async fn stream_turn_uses_poolside_chat_completions_with_tools_and_reasoning() {
        let server = spawn_chat_server(
            "/chat/completions",
            concat!(
                "data: {\"id\":\"chat-1\",\"choices\":[{\"delta\":{\"content\":\"\\n\\n\"},\"finish_reason\":null}]}\n\n",
                "data: {\"id\":\"chat-1\",\"choices\":[{\"delta\":{\"reasoning_content\":\"Thinking\"},\"finish_reason\":null}]}\n\n",
                "data: {\"id\":\"chat-1\",\"choices\":[{\"delta\":{\"content\":\"Hi\"},\"finish_reason\":null}]}\n\n",
                "data: {\"id\":\"chat-1\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call-1\",\"function\":{\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\"}}]},\"finish_reason\":null}]}\n\n",
                "data: {\"id\":\"chat-1\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"date\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
                "data: {\"choices\":[],\"usage\":{\"input_tokens\":3,\"output_tokens\":4,\"totalTokens\":7}}\n\n",
                "data: [DONE]\n\n",
            ),
        )
        .await;
        let engine = PoolsideInferenceEngine::new(PoolsideConfig {
            api_key: Some("secret".to_string()),
            base_url: Some(server.base_url.clone()),
        });
        let request = AgentInferenceRequest {
            model: ModelSelection {
                provider: PROVIDER_POOLSIDE.to_string(),
                model: "poolside/laguna-m.1".to_string(),
            },
            instructions: InstructionBundle::default(),
            transcript: vec![TranscriptItem::UserMessage(UserMessage::text("hi"))],
            tools: vec![ToolSpec {
                name: "exec_command".to_string(),
                description: "Run a command".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": { "cmd": { "type": "string" } },
                    "required": ["cmd"]
                }),
            }],
            tool_choice: ToolChoice::Auto,
            reasoning: ReasoningConfig {
                enabled: true,
                level: Some("medium".to_string()),
            },
            output: OutputConfig::default(),
            runtime: RuntimeHints {
                parallel_tool_calls: Some(false),
                ..RuntimeHints::default()
            },
            metadata: json!({}),
        };

        let mut stream = engine
            .stream_turn(
                InferenceTurnContext {
                    thread_id: "thread-1",
                    turn_id: "turn-1",
                    tool_executor: None,
                },
                request,
            )
            .await
            .unwrap();
        let mut events = Vec::new();
        while let Some(event) = stream.next().await {
            events.push(event.unwrap());
        }
        let request_body = server.request_body.await.unwrap();

        assert_eq!(request_body["model"], "poolside/laguna-m.1");
        assert_eq!(request_body["stream"], true);
        assert_eq!(request_body["stream_options"]["include_usage"], true);
        assert_eq!(
            request_body["chat_template_kwargs"]["enable_thinking"],
            true
        );
        assert!(request_body.get("reasoning").is_none());
        assert_eq!(request_body["parallel_tool_calls"], false);
        assert!(request_body.get("tools").is_some());
        assert!(matches!(
            events.iter().find(|event| matches!(event, InferenceEvent::ReasoningDelta(_))),
            Some(InferenceEvent::ReasoningDelta(delta)) if delta.text == "Thinking"
        ));
        assert!(matches!(
            events.iter().find(|event| matches!(event, InferenceEvent::MessageDelta(_))),
            Some(InferenceEvent::MessageDelta(delta)) if delta.text == "Hi"
        ));
        assert!(matches!(
            events.iter().find(|event| matches!(event, InferenceEvent::Usage(_))),
            Some(InferenceEvent::Usage(usage))
                if usage.prompt_tokens == 3
                    && usage.completion_tokens == 4
                    && usage.total_tokens == 7
        ));
        assert!(matches!(
            events.iter().find(|event| matches!(event, InferenceEvent::ToolCallCompleted(_))),
            Some(InferenceEvent::ToolCallCompleted(call))
                if call.name == "exec_command" && call.arguments == "{\"cmd\":\"date\"}"
        ));
    }

    #[tokio::test]
    async fn stream_turn_disables_poolside_thinking_for_none_reasoning() {
        let server = spawn_chat_server(
            "/chat/completions",
            concat!(
                "data: {\"id\":\"chat-1\",\"choices\":[{\"delta\":{\"content\":\"Hi\"},\"finish_reason\":\"stop\"}]}\n\n",
                "data: [DONE]\n\n",
            ),
        )
        .await;
        let engine = PoolsideInferenceEngine::new(PoolsideConfig {
            api_key: Some("secret".to_string()),
            base_url: Some(server.base_url.clone()),
        });
        let request = AgentInferenceRequest {
            model: ModelSelection {
                provider: PROVIDER_POOLSIDE.to_string(),
                model: "poolside/laguna-m.1".to_string(),
            },
            instructions: InstructionBundle::default(),
            transcript: vec![TranscriptItem::UserMessage(UserMessage::text("hi"))],
            tools: Vec::new(),
            tool_choice: ToolChoice::Auto,
            reasoning: ReasoningConfig {
                enabled: true,
                level: Some("none".to_string()),
            },
            output: OutputConfig::default(),
            runtime: RuntimeHints::default(),
            metadata: json!({}),
        };

        let mut stream = engine
            .stream_turn(
                InferenceTurnContext {
                    thread_id: "thread-1",
                    turn_id: "turn-1",
                    tool_executor: None,
                },
                request,
            )
            .await
            .unwrap();
        while let Some(event) = stream.next().await {
            event.unwrap();
        }

        let request_body = server.request_body.await.unwrap();
        assert_eq!(
            request_body["chat_template_kwargs"]["enable_thinking"],
            false
        );
        assert!(request_body.get("reasoning").is_none());
    }

    struct CapturedChatServer {
        base_url: String,
        request_body: tokio::sync::oneshot::Receiver<Value>,
    }

    async fn spawn_chat_server(
        expected_path: &'static str,
        response_body: &'static str,
    ) -> CapturedChatServer {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = Vec::new();
            let mut chunk = vec![0_u8; 4096];
            let header_end = loop {
                let n = stream.read(&mut chunk).await.unwrap();
                assert!(n > 0, "client closed before headers");
                buf.extend_from_slice(&chunk[..n]);
                if let Some(pos) = find_header_end(&buf) {
                    break pos;
                }
            };
            let header = String::from_utf8_lossy(&buf[..header_end]);
            let content_length = header
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or_default();
            while buf.len().saturating_sub(header_end + 4) < content_length {
                let n = stream.read(&mut chunk).await.unwrap();
                assert!(n > 0, "client closed before body");
                buf.extend_from_slice(&chunk[..n]);
            }
            let request = String::from_utf8_lossy(&buf);
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("");
            assert_eq!(path, expected_path);
            let body_start = header_end + 4;
            let body = String::from_utf8_lossy(&buf[body_start..body_start + content_length]);
            tx.send(serde_json::from_str(&body).unwrap()).unwrap();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{response_body}",
                response_body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });
        CapturedChatServer {
            base_url: format!("http://{addr}"),
            request_body: rx,
        }
    }

    fn find_header_end(buf: &[u8]) -> Option<usize> {
        buf.windows(4).position(|window| window == b"\r\n\r\n")
    }
}
