//! Hosted distribution acceptance (phase 72, Task 5): launches the hosted
//! gateway from a `HostedConfig` with fake auth and a fake-runtime tenant
//! factory, rejects unauthenticated WebSocket calls, and answers
//! authenticated `initialize` + `hosted/whoami`. Fully offline.

use std::sync::Arc;

use futures::{SinkExt, StreamExt};
use roder_api::extension::ExtensionRegistryBuilder;
use roder_config::hosted::HostedConfig;
use roder_core::fake_provider::FakeInferenceEngine;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

#[tokio::test(flavor = "multi_thread")]
async fn hosted_service_launches_with_fake_auth_and_answers_initialize() {
    let data_root = std::env::temp_dir().join(format!("roder-dist-{}", uuid::Uuid::new_v4()));
    // SAFETY: test-scoped env vars set before launch reads them.
    unsafe { std::env::set_var("RODER_DIST_TEST_KEY", "rk_test_dist_acceptance") };

    let config: HostedConfig = toml::from_str(&format!(
        r#"
        listen = "127.0.0.1:0"
        data_root = "{}"

        [[tenants]]
        id = "acme"

        [[static_keys]]
        token_env = "RODER_DIST_TEST_KEY"
        tenant = "acme"
        user = "ops"
        role = "tenant_admin"
        scopes = ["read", "write", "admin"]
        "#,
        data_root.display()
    ))
    .unwrap();

    // Fake runner/runtime profile: mock engine, no providers, no stores
    // beyond the tenant dir.
    let factory: roder_app_server_hosted::runtime_pool::TenantAppServerFactory =
        Arc::new(|_tenant, data_dir| {
            Box::pin(async move {
                let mut builder = ExtensionRegistryBuilder::new();
                builder.inference_engine(Arc::new(FakeInferenceEngine));
                builder.thread_store_factory(Arc::new(
                    roder_ext_jsonl_thread_store::store::JsonlThreadStoreFactory {
                        base_path: data_dir.join("threads"),
                    },
                ));
                let runtime = Arc::new(roder_core::Runtime::new(
                    builder.build().unwrap(),
                    roder_core::RuntimeConfig::default(),
                )?);
                Ok(Arc::new(roder_app_server::AppServer::new(runtime)))
            })
        });

    let controller = roder_dist_hosted::launch(config, factory).await.unwrap();
    let url = format!("ws://{}", controller.listen_addr);

    // Unauthenticated and bad-token connections fail at handshake.
    let request = url.clone().into_client_request().unwrap();
    assert!(tokio_tungstenite::connect_async(request).await.is_err());
    let mut bad = url.clone().into_client_request().unwrap();
    bad.headers_mut()
        .insert("authorization", "Bearer rk_test_wrong".parse().unwrap());
    assert!(tokio_tungstenite::connect_async(bad).await.is_err());

    // Authenticated initialize + whoami succeed.
    let mut good = url.clone().into_client_request().unwrap();
    good.headers_mut().insert(
        "authorization",
        "Bearer rk_test_dist_acceptance".parse().unwrap(),
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(good).await.unwrap();
    for (method, check) in [
        ("initialize", None),
        ("hosted/whoami", Some(("acme", "ops"))),
    ] {
        socket
            .send(Message::Text(
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": method,
                    "method": method,
                    "params": {}
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        let response = loop {
            let message = tokio::time::timeout(std::time::Duration::from_secs(10), socket.next())
                .await
                .expect("timeout")
                .expect("closed")
                .unwrap();
            if let Message::Text(text) = message {
                let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                if value.get("id").is_some_and(|id| !id.is_null()) {
                    break value;
                }
            }
        };
        assert!(response.get("error").is_none(), "{method}: {response}");
        if let Some((tenant, user)) = check {
            assert_eq!(response["result"]["tenant"]["tenantId"], tenant);
            assert_eq!(response["result"]["principal"]["user_id"], user);
        }
    }

    // Audit JSONL landed under the data root without raw tokens.
    controller.stop().await.unwrap();
    let audit = std::fs::read_to_string(data_root.join("audit.jsonl")).unwrap();
    assert!(audit.contains("auth_ok"));
    assert!(audit.contains("auth_failed"));
    assert!(!audit.contains("rk_test_dist_acceptance"));
    let _ = std::fs::remove_dir_all(&data_root);
}
