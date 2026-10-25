use super::*;

async fn unload_runner(fail_once: bool) {
    let sessions = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let provider = MockRunnerProvider::default();
    *provider.state.default_workspace.lock().unwrap() = Some("/runner/workspace".into());
    *provider.state.close_failures_remaining.lock().unwrap() = usize::from(fail_once);
    let mut builder = ExtensionRegistryBuilder::new();
    builder.inference_engine(Arc::new(FinalEngine {
        requests: Mutex::new(0),
    }));
    builder.thread_store_factory(Arc::new(JsonlThreadStoreFactory {
        base_path: sessions.path().to_path_buf(),
    }));
    builder.remote_runner_provider(Arc::new(provider.clone()));
    let runtime = Arc::new(
        Runtime::new(
            builder.build().unwrap(),
            RuntimeConfig {
                workspace: Some(workspace.path().display().to_string()),
                remote_runner_destination: Some(RunnerDestination {
                    id: "mock-hosted".into(),
                    provider_id: "mock-hosted".into(),
                    config: serde_json::Value::Null,
                    default_manifest: RunnerManifest::default(),
                }),
                ..Default::default()
            },
        )
        .unwrap(),
    );
    let thread = runtime.create_thread(None).await.unwrap();
    runtime
        .resume_thread_runner(&thread.thread_id)
        .await
        .unwrap();
    let mut events = runtime.subscribe_events();
    start_and_wait(&runtime, &mut events, &thread.thread_id, "first").await;
    assert_eq!(*provider.state.created.lock().unwrap(), 1);
    if fail_once {
        assert!(
            runtime
                .unload_thread(&thread.thread_id)
                .await
                .unwrap_err()
                .to_string()
                .contains("test close failure")
        );
    }
    runtime.unload_thread(&thread.thread_id).await.unwrap();
    assert_eq!(
        *provider.state.closed.lock().unwrap(),
        1 + usize::from(fail_once)
    );
    assert!(
        runtime
            .load_thread_metadata(&thread.thread_id)
            .await
            .unwrap()
            .is_some(),
        "unload preserves the conversation"
    );
    runtime
        .resume_thread_runner(&thread.thread_id)
        .await
        .unwrap();
    start_and_wait(&runtime, &mut events, &thread.thread_id, "resume").await;
    assert_eq!(
        *provider.state.resumed.lock().unwrap(),
        1,
        "a later turn reattaches instead of using the closed cached runner"
    );
}

#[tokio::test]
async fn unload_closes_runner_and_preserves_thread_for_reattachment() {
    unload_runner(false).await;
}

#[tokio::test]
async fn unload_retains_runner_for_retry_when_backend_close_fails() {
    unload_runner(true).await;
}
