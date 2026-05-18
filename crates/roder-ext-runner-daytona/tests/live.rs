#[tokio::test]
#[ignore = "requires RODER_LIVE_DAYTONA_RUNNER=1 and DAYTONA_API_KEY"]
async fn live_smoke() {
    roder_ext_runner_hosted_common::run_live_smoke_if_enabled(
        roder_ext_runner_daytona::daytona_runner_spec(),
    )
    .await;
}
