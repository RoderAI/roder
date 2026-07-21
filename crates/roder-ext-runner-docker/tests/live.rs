use roder_api::remote_runner::{
    RemoteRunnerProvider, RunnerCommandRequest, RunnerDestination, RunnerFileReadRequest,
    RunnerFileWriteRequest, RunnerManifest,
};
use roder_ext_runner_docker::DockerRunnerProvider;

#[tokio::test]
#[ignore = "requires Docker and RODER_LIVE_DOCKER_RUNNER=1"]
async fn live_docker_runner_reads_writes_runs_and_closes() {
    if std::env::var("RODER_LIVE_DOCKER_RUNNER").ok().as_deref() != Some("1") {
        eprintln!("set RODER_LIVE_DOCKER_RUNNER=1 to run live Docker runner tests");
        return;
    }
    let provider = DockerRunnerProvider;
    let session = provider
        .create_session(RunnerDestination {
            id: "docker-live".to_string(),
            provider_id: "docker".to_string(),
            config: serde_json::json!({ "image": "alpine:latest" }),
            default_manifest: RunnerManifest::default(),
        })
        .await
        .unwrap();

    session
        .write_file(RunnerFileWriteRequest {
            path: "hello.txt".into(),
            contents: b"hello docker\n".to_vec(),
        })
        .await
        .unwrap();
    let read = session
        .read_file(RunnerFileReadRequest {
            path: "hello.txt".into(),
        })
        .await
        .unwrap();
    assert_eq!(read.contents, b"hello docker\n");

    let output = session
        .run_command(RunnerCommandRequest {
            command_id: "cmd".to_string(),
            program: "cat".to_string(),
            args: vec!["hello.txt".to_string()],
            cwd: None,
            env: Vec::new(),
            timeout_ms: None,
        })
        .await
        .unwrap();
    assert_eq!(output.exit_code, Some(0));
    assert_eq!(output.stdout, "hello docker\n");

    session.close().await.unwrap();
}
