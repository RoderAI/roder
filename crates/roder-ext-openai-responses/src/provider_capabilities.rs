use super::*;

impl OpenAiResponsesEngine {
    /// Protocol support is independent of the mandatory-native policy used for
    /// first-party providers. Custom endpoints can opt in and retain fallback.
    pub fn supports_native_compaction(&self) -> bool {
        match self.provider_capabilities.remote_compaction {
            Some(RemoteCompaction::V2) => true,
            Some(RemoteCompaction::Unsupported) => false,
            None => self.requires_native_compaction(),
        }
    }

    pub(super) fn apply_provider_capabilities(&self, request: &mut AgentInferenceRequest) {
        if self.provider_capabilities.external_web_access == Some(false)
            && request.runtime.hosted_web_search.mode == HostedWebSearchMode::Live
        {
            request.runtime.hosted_web_search.mode = HostedWebSearchMode::Cached;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn protocol_support_is_distinct_from_native_requirement() {
        for (protocol, supported) in [
            (None, false),
            (Some(RemoteCompaction::Unsupported), false),
            (Some(RemoteCompaction::V2), true),
        ] {
            let engine = OpenAiResponsesEngine::new_custom_provider(
                None,
                "custom",
                "Custom",
                "https://custom.example/v1",
                ResponsesProviderCapabilities {
                    remote_compaction: protocol,
                    ..Default::default()
                },
            );
            assert_eq!(engine.supports_native_compaction(), supported);
            assert!(!engine.requires_native_compaction());
        }
        for provider in [PROVIDER_OPENAI, PROVIDER_CODEX] {
            let engine = OpenAiResponsesEngine::new_with_provider_id(None, provider);
            assert!(engine.supports_native_compaction());
            assert!(engine.requires_native_compaction());
        }
    }

    #[test]
    fn web_ceiling_only_restricts_live_search_and_cannot_enable_disabled_search() {
        for ceiling in [None, Some(false), Some(true)] {
            let engine = OpenAiResponsesEngine::new_custom_provider(
                None,
                "custom",
                "Custom",
                "https://custom.example/v1",
                ResponsesProviderCapabilities {
                    external_web_access: ceiling,
                    ..Default::default()
                },
            );
            for mode in [
                HostedWebSearchMode::Disabled,
                HostedWebSearchMode::Cached,
                HostedWebSearchMode::Live,
            ] {
                let mut request = crate::provider::tests::request();
                request.runtime.hosted_web_search.mode = mode;
                engine.apply_provider_capabilities(&mut request);
                let expected = if mode == HostedWebSearchMode::Live && ceiling == Some(false) {
                    HostedWebSearchMode::Cached
                } else {
                    mode
                };
                assert_eq!(request.runtime.hosted_web_search.mode, expected);
                let body = OpenAiResponsesEngine::map_request(&request);
                let search = body["tools"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|tool| tool["type"] == "web_search");
                if expected == HostedWebSearchMode::Disabled {
                    assert!(search.is_none());
                } else {
                    assert_eq!(
                        search.unwrap()["external_web_access"],
                        expected == HostedWebSearchMode::Live
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn ordinary_custom_turn_enforces_web_ceiling_on_the_wire() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut buf = [0; 4096];
            let header_end = loop {
                let n = socket.read(&mut buf).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
                if let Some(end) = bytes.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let length: usize = String::from_utf8_lossy(&bytes[..header_end])
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|value| value.trim().parse().ok())
                })
                .unwrap();
            while bytes.len() < header_end + length {
                let n = socket.read(&mut buf).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
            }
            let body: Value = serde_json::from_slice(&bytes[header_end..]).unwrap();
            assert_eq!(body["tools"][0]["type"], "web_search");
            assert_eq!(body["tools"][0]["external_web_access"], false);
            let event = json!({
                "type":"response.completed",
                "response":{"id":"custom-turn","output":[]}
            });
            let sse = format!("data: {event}\n\n");
            socket.write_all(format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{sse}",
                sse.len()
            ).as_bytes()).await.unwrap();
        });
        let engine = OpenAiResponsesEngine::new_custom_provider(
            Some("fixture-key".into()),
            "custom",
            "Custom",
            url,
            ResponsesProviderCapabilities {
                external_web_access: Some(false),
                ..Default::default()
            },
        );
        let mut request = crate::provider::tests::request();
        request.model.provider = "custom".into();
        request.runtime.hosted_web_search.mode = HostedWebSearchMode::Live;
        let mut stream = engine
            .stream_turn(
                InferenceTurnContext {
                    thread_id: "thread",
                    turn_id: "turn",
                    tool_executor: None,
                },
                request,
            )
            .await
            .unwrap();
        let mut completed = false;
        while let Some(event) = stream.next().await {
            completed |= matches!(event.unwrap(), InferenceEvent::Completed(_));
        }
        assert!(completed);
        server.await.unwrap();
    }
}
