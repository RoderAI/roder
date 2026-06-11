//! Hosted startup validation (phase 72, Task 7): misconfiguration fails
//! closed before the gateway binds — missing auth env vars, missing
//! tenants, raw inline keys, and insecure-by-accident settings.

use roder_config::hosted::HostedConfig;

fn base_config(data_root: &str) -> String {
    format!(
        r#"
        listen = "127.0.0.1:0"
        data_root = "{data_root}"

        [[tenants]]
        id = "acme"

        [[static_keys]]
        token_env = "RODER_DIST_VALIDATION_UNSET_KEY"
        tenant = "acme"
        user = "ops"
        "#
    )
}

#[tokio::test]
async fn hosted_startup_validation_fails_closed_on_missing_auth_env() {
    let data_root = std::env::temp_dir().join(format!("roder-sv-{}", uuid::Uuid::new_v4()));
    let config: HostedConfig =
        toml::from_str(&base_config(&data_root.display().to_string())).unwrap();
    // The referenced env var is deliberately unset.
    let error = match roder_dist_hosted::launch(
        config,
        roder_dist_hosted::default_tenant_factory(),
    )
    .await
    {
        Ok(_) => panic!("launch must fail closed on a missing auth env var"),
        Err(error) => error.to_string(),
    };
    assert!(error.contains("RODER_DIST_VALIDATION_UNSET_KEY"), "{error}");
    assert!(error.contains("refusing to start"), "{error}");
    let _ = std::fs::remove_dir_all(&data_root);
}

#[test]
fn hosted_startup_validation_rejects_bad_configs() {
    // Missing tenants.
    let config: HostedConfig = toml::from_str(
        r#"
        data_root = "/tmp/x"
        "#,
    )
    .unwrap();
    assert!(config.validate().unwrap_err().to_string().contains("tenants"));

    // Empty data root.
    let config: HostedConfig = toml::from_str(
        r#"
        data_root = ""
        [[tenants]]
        id = "acme"
        "#,
    )
    .unwrap();
    assert!(config.validate().unwrap_err().to_string().contains("data_root"));

    // Raw inline key material.
    let config: HostedConfig = toml::from_str(
        r#"
        data_root = "/tmp/x"
        [[tenants]]
        id = "acme"
        [[static_keys]]
        token_env = "rk_test_inline_raw_key"
        tenant = "acme"
        user = "ops"
        "#,
    )
    .unwrap();
    assert!(
        config
            .validate()
            .unwrap_err()
            .to_string()
            .contains("never accepted")
    );
}

/**
 * Live deployment smoke (opt-in): points at a running hosted deployment
 * and proves auth + whoami end to end.
 *
 * ```sh
 * RODER_HOSTED_LIVE=1 RODER_HOSTED_URL=ws://... RODER_HOSTED_TOKEN=rk_test_... \
 *   cargo test -p roder-dist-hosted hosted_live_smoke -- --ignored
 * ```
 */
#[tokio::test]
#[ignore = "requires RODER_HOSTED_LIVE=1 and a running hosted deployment"]
async fn hosted_live_smoke() {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    if std::env::var("RODER_HOSTED_LIVE").as_deref() != Ok("1") {
        eprintln!("RODER_HOSTED_LIVE not set; skipping hosted live smoke");
        return;
    }
    let url = std::env::var("RODER_HOSTED_URL").expect("RODER_HOSTED_URL");
    let token = std::env::var("RODER_HOSTED_TOKEN").expect("RODER_HOSTED_TOKEN");

    let mut request = url.into_client_request().unwrap();
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    socket
        .send(Message::Text(
            serde_json::json!({"jsonrpc":"2.0","id":1,"method":"hosted/whoami","params":{}})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let response = loop {
        if let Some(Ok(Message::Text(text))) = socket.next().await {
            let value: serde_json::Value = serde_json::from_str(&text).unwrap();
            if value.get("id").is_some_and(|id| !id.is_null()) {
                break value;
            }
        }
    };
    assert!(response.get("error").is_none(), "{response}");
    eprintln!("hosted live smoke ok: {}", response["result"]["tenant"]["tenantId"]);
}
