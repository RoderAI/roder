#[tokio::test]
#[ignore = "requires RODER_LIVE_BLAXEL_RUNNER=1, BLAXEL_API_KEY and BL_WORKSPACE"]
async fn live_smoke() {
    roder_ext_runner_blaxel::run_live_smoke_if_enabled()
        .await
        .unwrap();
}
