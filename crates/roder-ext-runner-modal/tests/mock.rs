#[tokio::test]
async fn mock_lifecycle_matches_runner_contract() {
    roder_ext_runner_hosted_common::run_mock_lifecycle_test(
        roder_ext_runner_modal::modal_runner_spec(),
    )
    .await;
}
