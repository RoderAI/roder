use super::{OpenAiResponsesEngine, PROVIDER_OPENAI};
use serde_json::{Value, json};

impl OpenAiResponsesEngine {
    /// Set the default OpenAI API service tier, such as `ultrafast`.
    /// Per-request runtime hints take precedence. Other providers are unaffected.
    pub fn with_service_tier(mut self, service_tier: Option<String>) -> Self {
        if self.provider_id == PROVIDER_OPENAI {
            self.service_tier = service_tier
                .map(|tier| tier.trim().to_string())
                .filter(|tier| !tier.is_empty());
        }
        self
    }

    pub(super) fn apply_service_tier(&self, body: &mut Value) {
        if body.get("service_tier").is_none()
            && let Some(tier) = &self.service_tier
        {
            body["service_tier"] = json!(tier);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_default_yields_to_request_override() {
        let engine = OpenAiResponsesEngine::new(None).with_service_tier(Some("ultrafast".into()));
        let mut body = json!({"model": "gpt-6.1-sol"});
        engine.apply_service_tier(&mut body);
        assert_eq!(body["service_tier"], "ultrafast");

        let mut body = json!({"service_tier": "default"});
        engine.apply_service_tier(&mut body);
        assert_eq!(body["service_tier"], "default");
    }

    #[test]
    fn absent_or_blank_default_leaves_request_unchanged() {
        for tier in [None, Some("  ".into())] {
            let engine = OpenAiResponsesEngine::new(None).with_service_tier(tier);
            let mut body = json!({});
            engine.apply_service_tier(&mut body);
            assert_eq!(body, json!({}));
        }
    }

    #[test]
    fn openai_default_does_not_leak_to_other_providers() {
        for provider in ["codex", "openrouter", "xai", "fireworks", "custom"] {
            let engine = OpenAiResponsesEngine::new_with_provider_id(None, provider)
                .with_service_tier(Some("ultrafast".into()));
            let mut body = json!({});
            engine.apply_service_tier(&mut body);
            assert_eq!(body, json!({}), "{provider}");
        }
    }

    #[tokio::test]
    async fn configured_tier_reaches_http_request_and_runtime_hint_wins() {
        use roder_api::inference::{InferenceEngine, InferenceTurnContext};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        for override_tier in [None, Some("default")] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let body = loop {
                    let mut chunk = [0; 4096];
                    let count = socket.read(&mut chunk).await.unwrap();
                    assert_ne!(count, 0);
                    bytes.extend_from_slice(&chunk[..count]);
                    let Some(end) = bytes.windows(4).position(|chunk| chunk == b"\r\n\r\n") else {
                        continue;
                    };
                    let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                    let length: usize = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    if bytes.len() >= end + 4 + length {
                        break serde_json::from_slice::<Value>(&bytes[end + 4..end + 4 + length])
                            .unwrap();
                    }
                };
                socket.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
                body
            });
            let engine = OpenAiResponsesEngine::new_with_config(
                Some("test-key".into()),
                PROVIDER_OPENAI,
                format!("http://{address}"),
                Vec::new(),
            )
            .with_service_tier(Some("ultrafast".into()));
            let mut request = super::super::tests::request();
            request.runtime.service_tier = override_tier.map(str::to_string);
            let _ = engine
                .stream_turn(
                    InferenceTurnContext {
                        thread_id: "thread",
                        turn_id: "turn",
                        tool_executor: None,
                    },
                    request,
                )
                .await;
            let body = server.await.unwrap();
            assert_eq!(body["service_tier"], override_tier.unwrap_or("ultrafast"));
        }
    }
}
