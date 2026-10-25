use super::*;
use roder_app_server::acp::AcpAuthentication;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
struct RecordingAuthentication {
    calls: AtomicUsize,
    fail: bool,
}

#[async_trait]
impl AcpAuthentication for RecordingAuthentication {
    fn methods(&self) -> Vec<acp::AuthMethod> {
        vec![acp::AuthMethod::Agent(acp::AuthMethodAgent::new(
            "browser-login",
            "Sign in",
        ))]
    }

    async fn is_authenticated(&self) -> anyhow::Result<bool> {
        Ok(self.calls.load(Ordering::SeqCst) > 0 && !self.fail)
    }

    async fn authenticate(&self, method_id: &str) -> anyhow::Result<()> {
        assert_eq!(method_id, "browser-login");
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            anyhow::bail!("browser authentication failed");
        }
        Ok(())
    }
}

async fn request(
    adapter: &AcpAdapter<LocalAppClient>,
    method: &str,
    params: serde_json::Value,
) -> roder_protocol::JsonRpcResponse {
    adapter
        .handle_request(
            JsonRpcRequest {
                jsonrpc: "2.0".into(),
                id: Some(serde_json::json!("auth-test")),
                method: method.into(),
                params: Some(params),
            },
            &RecordingPeer::default(),
        )
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn initialize_advertises_the_host_browser_auth_and_authenticate_invokes_it() {
    let auth = Arc::new(RecordingAuthentication::default());
    let adapter = test_adapter()
        .with_authentication(auth.clone())
        .with_agent_info(acp::Implementation::new("test-host", "0.3.19").title("Test host"));
    let initialized = request(
        &adapter,
        "initialize",
        serde_json::json!({"protocolVersion": 1}),
    )
    .await;
    let result = initialized.result.unwrap();
    assert_eq!(result["agentInfo"]["name"], "test-host");
    assert_eq!(result["agentInfo"]["version"], "0.3.19");
    assert_eq!(result["authMethods"][0]["id"], "browser-login");
    assert_eq!(result["authMethods"][0]["name"], "Sign in");
    assert_eq!(result["agentCapabilities"]["loadSession"], true);
    assert_eq!(auth.calls.load(Ordering::SeqCst), 0);

    let response = request(
        &adapter,
        "authenticate",
        serde_json::json!({"methodId": "browser-login"}),
    )
    .await;
    assert!(response.error.is_none());
    assert_eq!(response.result, Some(serde_json::json!({})));
    assert_eq!(auth.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn authenticate_rejects_unknown_or_malformed_methods_before_starting_login() {
    let auth = Arc::new(RecordingAuthentication::default());
    let adapter = test_adapter().with_authentication(auth.clone());
    for params in [
        serde_json::json!({"methodId": "unknown"}),
        serde_json::json!({}),
    ] {
        let response = request(&adapter, "authenticate", params).await;
        assert_eq!(response.error.unwrap().code, -32602);
    }
    assert_eq!(auth.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn authenticate_returns_host_login_failures() {
    let auth = Arc::new(RecordingAuthentication {
        fail: true,
        ..Default::default()
    });
    let adapter = test_adapter().with_authentication(auth.clone());
    let response = request(
        &adapter,
        "authenticate",
        serde_json::json!({"methodId": "browser-login"}),
    )
    .await;
    let error = response.error.unwrap();
    assert_eq!(error.code, -32603);
    assert!(error.message.contains("browser authentication failed"));
    assert_eq!(auth.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn adapter_without_a_host_authenticator_does_not_accept_login() {
    let response = request(
        &test_adapter(),
        "authenticate",
        serde_json::json!({"methodId": "browser-login"}),
    )
    .await;
    assert_eq!(response.error.unwrap().code, -32601);
}

#[tokio::test]
async fn session_new_requires_authentication_and_works_after_login() {
    let auth = Arc::new(RecordingAuthentication::default());
    let adapter = test_adapter().with_authentication(auth);
    let params = serde_json::json!({"cwd": std::env::current_dir().unwrap(), "mcpServers": []});
    let rejected = request(&adapter, "session/new", params.clone()).await;
    let error = rejected.error.unwrap();
    assert_eq!(error.code, -32000);
    assert_eq!(error.message, "Authentication required");
    request(
        &adapter,
        "authenticate",
        serde_json::json!({"methodId": "browser-login"}),
    )
    .await;
    let created = request(&adapter, "session/new", params).await;
    assert!(created.error.is_none(), "{:?}", created.error);
    assert!(created.result.unwrap()["sessionId"].is_string());
}

#[tokio::test]
async fn initialize_with_a_different_protocol_version_returns_supported_v1() {
    let initialized = request(
        &test_adapter(),
        "initialize",
        serde_json::json!({"protocolVersion": 999}),
    )
    .await;
    assert_eq!(initialized.result.unwrap()["protocolVersion"], 1);
}
