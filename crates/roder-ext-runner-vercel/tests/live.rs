#[tokio::test]
#[ignore = "requires RODER_LIVE_VERCEL_RUNNER=1 and VERCEL_TOKEN"]
async fn live_smoke() {
    roder_ext_runner_hosted_common::run_live_smoke_if_enabled(
        roder_ext_runner_vercel::vercel_runner_spec(),
    )
    .await;
}
