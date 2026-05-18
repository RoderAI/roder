#[tokio::test]
#[ignore = "requires RODER_LIVE_BLAXEL_RUNNER=1 and BLAXEL_API_KEY"]
async fn live_smoke() {
    roder_ext_runner_hosted_common::run_live_smoke_if_enabled(
        roder_ext_runner_blaxel::blaxel_runner_spec(),
    )
    .await;
}
