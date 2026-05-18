#[tokio::test]
#[ignore = "requires RODER_LIVE_CLOUDFLARE_RUNNER=1 and CLOUDFLARE_API_TOKEN"]
async fn live_smoke() {
    roder_ext_runner_hosted_common::run_live_smoke_if_enabled(
        roder_ext_runner_cloudflare::cloudflare_runner_spec(),
    )
    .await;
}
