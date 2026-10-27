use super::*;

#[tokio::test]
async fn shared_read_classification_preserves_remote_session_and_provider_fences() {
    let session_dir = temp_dir("runner-shared-read-fence-sessions");
    let scratch = temp_dir("runner-shared-read-fence-scratch");
    let provider = MockRunnerProvider::default();
    provider
        .state
        .workspace_leases_enabled
        .store(true, Ordering::SeqCst);
    provider
        .state
        .synchronize_reads
        .store(true, Ordering::SeqCst);
    provider
        .state
        .pause_first_read
        .store(true, Ordering::SeqCst);
    provider
        .state
        .files
        .lock()
        .unwrap()
        .insert("shared.txt".into(), b"contents\n".to_vec());
    let runtime = coding_tools_runtime(
        session_dir.clone(),
        scratch.clone(),
        provider.clone(),
        vec![tool_call_round(&[
            ("read-a", "read_file", r#"{"path":"shared.txt"}"#),
            ("read-b", "read_file", r#"{"path":"shared.txt"}"#),
        ])],
    );
    let thread = runtime
        .create_thread_with(mock_runner_thread_request(&scratch))
        .await
        .unwrap();
    let started = provider.state.first_read_started.notified();
    tokio::pin!(started);
    started.as_mut().enable();
    let execution = tokio::spawn({
        let runtime = runtime.clone();
        let scratch = scratch.clone();
        async move { run_turn_collecting_tool_calls(&runtime, &thread.thread_id, &scratch).await }
    });
    tokio::time::timeout(Duration::from_secs(5), started)
        .await
        .expect("the first remote read must reach its pause point");
    assert_eq!(provider.state.reads_started.load(Ordering::SeqCst), 1);
    assert_eq!(
        provider
            .state
            .workspace_lease_acquisitions
            .load(Ordering::SeqCst),
        1,
        "the first read holds the provider fence"
    );
    assert!(
        tokio::time::timeout(
            Duration::from_millis(100),
            provider.state.second_read_started.notified(),
        )
        .await
        .is_err(),
        "read metadata must not bypass remote compound-operation serialization"
    );
    assert_eq!(provider.state.reads_started.load(Ordering::SeqCst), 1);
    provider.state.release_first_read.notify_one();
    let results = tokio::time::timeout(Duration::from_secs(5), execution)
        .await
        .expect("both remote reads finish after the first releases its fence")
        .unwrap();
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|result| !result.is_error), "{results:?}");
    assert_eq!(provider.state.reads_started.load(Ordering::SeqCst), 2);
    assert_eq!(
        provider
            .state
            .workspace_lease_acquisitions
            .load(Ordering::SeqCst),
        2
    );
    assert_eq!(
        provider
            .state
            .workspace_lease_releases
            .load(Ordering::SeqCst),
        2
    );
    let _ = std::fs::remove_dir_all(session_dir);
    let _ = std::fs::remove_dir_all(scratch);
}
