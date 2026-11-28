use super::*;
use roder_protocol::SettingsSetModelContextWindowResult;
use serde_json::json;

#[tokio::test]
async fn context_window_settings_persist_reset_and_reject_invalid_values() {
    let _guard = RODER_CONFIG_DIR_TEST_LOCK.lock().await;
    let dir = std::env::temp_dir().join(format!("roder-context-window-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let _config_dir = EnvVarGuard::set("RODER_CONFIG_DIR", &dir);
    std::fs::write(dir.join("config.toml"), "model = 'gpt-6.2-sol'\n").unwrap();
    let runtime = Arc::new(Runtime::fake().unwrap());
    let client = LocalAppClient::new(Arc::new(
        AppServer::new(runtime.clone()).with_user_config_persistence(),
    ));
    let saved: SettingsSetModelContextWindowResult = request(
        &client,
        "settings/set_model_context_window",
        Some(json!({"model_context_window": 200000})),
    )
    .await;
    assert_eq!(saved.model_context_window, Some(200_000));
    assert_eq!(
        saved.config_path,
        dir.join("config.toml").display().to_string()
    );
    let cfg = std::fs::read_to_string(&saved.config_path).unwrap();
    assert!(cfg.contains("model_context_window = 200000"));
    assert!(cfg.contains("model = \"gpt-6.2-sol\""));
    let current: SettingsGetResult = request(&client, "settings/get", None).await;
    assert_eq!(current.model_context_window, Some(200_000));
    for invalid in [json!(0), json!(-1), json!(4294967296_u64), json!("500k")] {
        let error = request_error(
            &client,
            "settings/set_model_context_window",
            Some(json!({"model_context_window": invalid})),
        )
        .await;
        assert_eq!(error.code, -32602);
        assert_eq!(runtime.status().await.model_context_window, Some(200_000));
    }
    let reset: SettingsSetModelContextWindowResult = request(
        &client,
        "settings/set_model_context_window",
        Some(json!({"model_context_window": null})),
    )
    .await;
    assert_eq!(reset.model_context_window, None);
    assert!(
        !std::fs::read_to_string(&saved.config_path)
            .unwrap()
            .contains("model_context_window")
    );
    assert_eq!(runtime.status().await.model_context_window, None);
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn context_window_persistence_failure_leaves_live_override_unchanged() {
    let _guard = RODER_CONFIG_DIR_TEST_LOCK.lock().await;
    let dir = std::env::temp_dir().join(format!(
        "roder-context-window-failure-{}",
        uuid::Uuid::new_v4()
    ));
    // A read-only config makes the save fail while staying readable: other
    // tests in this process read whichever config dir is set at the moment.
    std::fs::create_dir_all(&dir).unwrap();
    let config = dir.join("config.toml");
    std::fs::write(&config, "").unwrap();
    let mut permissions = std::fs::metadata(&config).unwrap().permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&config, permissions).unwrap();
    let _config_dir = EnvVarGuard::set("RODER_CONFIG_DIR", &dir);
    let runtime = Arc::new(Runtime::fake().unwrap());
    runtime
        .set_model_context_window(Some(200_000))
        .await
        .unwrap();
    let client = LocalAppClient::new(Arc::new(
        AppServer::new(runtime.clone()).with_user_config_persistence(),
    ));
    let error = request_error(
        &client,
        "settings/set_model_context_window",
        Some(json!({"model_context_window": 300000})),
    )
    .await;
    assert_eq!(error.code, -32000);
    assert_eq!(runtime.status().await.model_context_window, Some(200_000));
    assert_eq!(std::fs::read_to_string(&config).unwrap(), "");
    std::fs::remove_dir_all(dir).unwrap();
}
