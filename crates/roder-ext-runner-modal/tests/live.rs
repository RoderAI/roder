#[tokio::test]
#[ignore = "requires RODER_LIVE_MODAL_RUNNER=1 and MODAL_TOKEN_SECRET"]
async fn live_smoke() {
    roder_ext_runner_hosted_common::run_live_smoke_if_enabled(
        roder_ext_runner_modal::modal_runner_spec(),
    )
    .await;
}
