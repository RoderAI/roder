//! Gemini-on-Vertex inference engine. Bodies are Gemini API shapes
//! (`crate::mapping`); Vertex differs in auth (OAuth2 service-account token),
//! endpoint (regional/global `aiplatform.googleapis.com` hosts) and model
//! addressing (`publishers/google/models/...` paths).

use roder_api::catalog::models_for_provider;
use roder_api::extension::InferenceEngineId;
use roder_api::inference::{
    AgentInferenceRequest, InferenceCapabilities, InferenceEngine, InferenceEventStream,
    InferenceProviderContext, InferenceProviderMetadata, InferenceTurnContext, ModelDescriptor,
    ProviderAuthType,
};

use crate::auth::ServiceAccountTokenSource;
use crate::mapping::map_request;
use crate::stream::{VertexTurnRequest, start_vertex_stream, vertex_stream_client};

pub const VERTEX_PROVIDER_ID: &str = "vertex";
pub const PROJECT_ENV: &str = "VERTEX_PROJECT";
pub const LOCATION_ENV: &str = "VERTEX_LOCATION";

/// The global endpoint uses the bare host; regional locations get a
/// `{location}-` host prefix.
const GLOBAL_LOCATION: &str = "global";

#[derive(Clone, Default)]
pub struct VertexConfig {
    /// Path to a service-account JSON key file (`GOOGLE_APPLICATION_CREDENTIALS`).
    pub credentials_path: Option<String>,
    /// Inline service-account JSON (`VERTEX_CREDENTIALS_JSON`).
    pub credentials_json: Option<String>,
    /// `VERTEX_PROJECT`; falls back to the credentials' `project_id`.
    pub project: Option<String>,
    /// `VERTEX_LOCATION`; defaults to `global`.
    pub location: Option<String>,
}

impl std::fmt::Debug for VertexConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VertexConfig")
            .field("credentials_path", &self.credentials_path)
            .field(
                "credentials_json",
                &self.credentials_json.as_ref().map(|_| "<redacted>"),
            )
            .field("project", &self.project)
            .field("location", &self.location)
            .finish()
    }
}

pub struct VertexEngine {
    token_source: ServiceAccountTokenSource,
    project: Option<String>,
    location: Option<String>,
    /// Replaces the computed `aiplatform.googleapis.com` host (fake-server
    /// tests).
    endpoint_override: Option<String>,
}

impl VertexEngine {
    pub fn new(config: VertexConfig) -> Self {
        Self {
            token_source: ServiceAccountTokenSource::new(
                config.credentials_json,
                config.credentials_path,
                None,
            ),
            project: config.project,
            location: config.location,
            endpoint_override: None,
        }
    }

    fn resolved_project(&self) -> anyhow::Result<String> {
        self.project
            .clone()
            .or_else(|| self.token_source.project_from_credentials())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Vertex AI project is not configured; set {PROJECT_ENV} or use \
                     service-account credentials that carry a project_id"
                )
            })
    }

    fn model_url(&self, project: &str, location: &str, model: &str) -> String {
        let host = match self.endpoint_override.as_deref() {
            Some(endpoint) => endpoint.trim_end_matches('/').to_string(),
            None if location == GLOBAL_LOCATION => "https://aiplatform.googleapis.com".to_string(),
            None => format!("https://{location}-aiplatform.googleapis.com"),
        };
        format!(
            "{host}/v1/projects/{project}/locations/{location}/publishers/google/models/{model}:streamGenerateContent?alt=sse"
        )
    }
}

#[async_trait::async_trait]
impl InferenceEngine for VertexEngine {
    fn id(&self) -> InferenceEngineId {
        VERTEX_PROVIDER_ID.to_string()
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities {
            streaming: true,
            tool_calls: true,
            parallel_tool_calls: false,
            reasoning_summaries: false,
            structured_output: true,
            image_input: true,
            prompt_cache: false,
            provider_metadata: true,
            tool_search: false,
        }
    }

    fn metadata(&self) -> InferenceProviderMetadata {
        InferenceProviderMetadata {
            name: "Google Vertex AI".to_string(),
            description: Some("Gemini on Vertex AI with service-account credentials".to_string()),
            auth_type: ProviderAuthType::OAuth,
            auth_label: Some("Service-account JSON".to_string()),
            auth_configured: Some(self.token_source.looks_configured()),
            recommended: false,
            sort_order: 41,
        }
    }

    async fn list_models(
        &self,
        _ctx: InferenceProviderContext<'_>,
    ) -> anyhow::Result<Vec<ModelDescriptor>> {
        Ok(models_for_provider(VERTEX_PROVIDER_ID, false))
    }

    async fn stream_turn(
        &self,
        _ctx: InferenceTurnContext<'_>,
        request: AgentInferenceRequest,
    ) -> anyhow::Result<InferenceEventStream> {
        let body = map_request(&request)?;
        // Token minting first: when credentials are missing entirely the
        // error names the credential env vars instead of VERTEX_PROJECT.
        let access_token = self.token_source.access_token().await?;
        let project = self.resolved_project()?;
        let location = self.location.as_deref().unwrap_or(GLOBAL_LOCATION);
        let url = self.model_url(&project, location, &request.model.model);
        start_vertex_stream(VertexTurnRequest {
            client: vertex_stream_client()?,
            url,
            access_token,
            body,
            policy: request.runtime.reliability.clone().unwrap_or_default(),
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{CREDENTIALS_JSON_ENV, CREDENTIALS_PATH_ENV, test_credentials};
    use futures::StreamExt;
    use roder_api::inference::{
        InferenceEvent, InstructionBundle, ModelSelection, OutputConfig, ReasoningConfig,
        RuntimeHints,
    };
    use roder_api::tools::ToolChoice;
    use roder_api::transcript::{TranscriptItem, UserMessage};
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn engine(config: VertexConfig) -> VertexEngine {
        VertexEngine::new(config)
    }

    fn turn_ctx() -> InferenceTurnContext<'static> {
        InferenceTurnContext {
            thread_id: "thread",
            turn_id: "turn",
            tool_executor: None,
        }
    }

    fn request(model: &str) -> AgentInferenceRequest {
        AgentInferenceRequest {
            model: ModelSelection {
                provider: VERTEX_PROVIDER_ID.to_string(),
                model: model.to_string(),
            },
            instructions: InstructionBundle::default(),
            transcript: vec![TranscriptItem::UserMessage(UserMessage::text("Hello"))],
            tools: Vec::new(),
            tool_choice: ToolChoice::Auto,
            reasoning: ReasoningConfig::default(),
            output: OutputConfig::default(),
            runtime: RuntimeHints::default(),
            metadata: json!({}),
        }
    }

    #[test]
    fn model_url_uses_global_host_for_global_location() {
        let engine = engine(VertexConfig::default());
        assert_eq!(
            engine.model_url("proj", "global", "gemini-3.5-flash"),
            "https://aiplatform.googleapis.com/v1/projects/proj/locations/global/publishers/google/models/gemini-3.5-flash:streamGenerateContent?alt=sse"
        );
    }

    #[test]
    fn model_url_uses_regional_host_for_regional_location() {
        let engine = engine(VertexConfig::default());
        assert_eq!(
            engine.model_url("proj", "us-east5", "gemini-3.5-flash"),
            "https://us-east5-aiplatform.googleapis.com/v1/projects/proj/locations/us-east5/publishers/google/models/gemini-3.5-flash:streamGenerateContent?alt=sse"
        );
    }

    #[test]
    fn metadata_reports_auth_configured_from_credential_presence() {
        let unconfigured = engine(VertexConfig::default());
        assert_eq!(unconfigured.metadata().auth_configured, Some(false));

        let configured = engine(VertexConfig {
            credentials_json: Some(test_credentials::credentials_json()),
            ..VertexConfig::default()
        });
        assert_eq!(configured.metadata().auth_configured, Some(true));
        assert!(configured.capabilities().streaming);
        assert_eq!(configured.id(), "vertex");
    }

    #[test]
    fn config_debug_redacts_inline_credentials() {
        let config = VertexConfig {
            credentials_json: Some(test_credentials::credentials_json()),
            project: Some("proj".to_string()),
            ..VertexConfig::default()
        };

        let debug = format!("{config:?}");

        assert!(debug.contains("<redacted>"), "{debug}");
        assert!(!debug.contains("PRIVATE KEY"), "{debug}");
    }

    async fn stream_turn_error(engine: &VertexEngine) -> String {
        match engine
            .stream_turn(turn_ctx(), request("gemini-3.5-flash"))
            .await
        {
            Ok(_) => panic!("expected stream_turn to fail"),
            Err(err) => err.to_string(),
        }
    }

    #[tokio::test]
    async fn missing_credentials_fail_at_call_time_naming_env_vars() {
        let engine = engine(VertexConfig {
            project: Some("proj".to_string()),
            ..VertexConfig::default()
        });

        let err = stream_turn_error(&engine).await;

        assert!(err.contains(CREDENTIALS_PATH_ENV), "{err}");
        assert!(err.contains(CREDENTIALS_JSON_ENV), "{err}");
    }

    #[tokio::test]
    async fn missing_project_fails_at_call_time_naming_env_var() {
        let token_url = spawn_token_server().await;
        let credentials = json!({
            "type": "service_account",
            "client_email": "vertex-test@example.iam.gserviceaccount.com",
            "private_key": test_credentials::TEST_PRIVATE_KEY_PEM,
        })
        .to_string();
        let engine = VertexEngine {
            token_source: ServiceAccountTokenSource::new(Some(credentials), None, Some(token_url)),
            project: None,
            location: None,
            endpoint_override: None,
        };

        let err = stream_turn_error(&engine).await;

        assert!(err.contains(PROJECT_ENV), "{err}");
    }

    #[tokio::test]
    async fn streams_turn_with_minted_bearer_token_and_publisher_path() {
        let token_url = spawn_token_server().await;
        let (vertex_url, captured_request) = spawn_vertex_server(
            [
                r#"data: {"responseId":"resp_1","candidates":[{"content":{"parts":[{"text":"Hi"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":2,"candidatesTokenCount":1,"totalTokenCount":3}}"#,
            ]
            .map(|frame| format!("{frame}\n\n"))
            .join(""),
        )
        .await;
        let mut engine = VertexEngine::new(VertexConfig {
            credentials_json: Some(test_credentials::credentials_json()),
            location: Some("us-east5".to_string()),
            // project deliberately unset: resolved from credentials project_id
            ..VertexConfig::default()
        });
        engine.token_source = ServiceAccountTokenSource::new(
            Some(test_credentials::credentials_json()),
            None,
            Some(token_url),
        );
        engine.endpoint_override = Some(vertex_url);

        let stream = engine
            .stream_turn(turn_ctx(), request("gemini-3.5-flash"))
            .await
            .unwrap();
        let events = stream
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .map(Result::unwrap)
            .collect::<Vec<_>>();

        let raw_request = captured_request.lock().unwrap().clone();
        assert!(
            raw_request.contains(
                "POST /v1/projects/example-project/locations/us-east5/publishers/google/models/gemini-3.5-flash:streamGenerateContent?alt=sse"
            ),
            "{raw_request}"
        );
        assert!(
            raw_request
                .to_ascii_lowercase()
                .contains("authorization: bearer minted-token"),
            "{raw_request}"
        );
        assert!(events.contains(&InferenceEvent::MessageDelta(
            roder_api::inference::MessageDelta {
                text: "Hi".to_string(),
                phase: None,
            }
        )));
        assert!(matches!(
            events.last(),
            Some(InferenceEvent::Completed(metadata))
                if metadata.stop_reason.as_deref() == Some("stop")
        ));
    }

    /// Serves one token-exchange response granting `minted-token`.
    async fn spawn_token_server() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let mut buf = vec![0_u8; 65536];
            let _ = stream.read(&mut buf).await.unwrap();
            let body = json!({ "access_token": "minted-token", "expires_in": 3600 }).to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });
        format!("http://{addr}/token")
    }

    /// Serves one SSE response and captures the raw request head + body.
    async fn spawn_vertex_server(sse_body: String) -> (String, Arc<Mutex<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let captured = Arc::new(Mutex::new(String::new()));
        let server_captured = captured.clone();
        tokio::spawn(async move {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let mut buf = vec![0_u8; 65536];
            let n = stream.read(&mut buf).await.unwrap();
            *server_captured.lock().unwrap() = String::from_utf8_lossy(&buf[..n]).into_owned();
            let head =
                b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n";
            stream.write_all(head).await.unwrap();
            stream.write_all(sse_body.as_bytes()).await.unwrap();
        });
        (format!("http://{addr}"), captured)
    }
}
