#[tokio::test]
#[ignore = "requires RODER_XAI_LIVE=1 and XAI_API_KEY or SuperGrok OAuth tokens"]
async fn live_xai_tools_smoke_is_explicitly_opt_in() {
    if std::env::var("RODER_XAI_LIVE").ok().as_deref() != Some("1") {
        eprintln!("set RODER_XAI_LIVE=1 to run live xAI smoke tests");
        return;
    }

    let has_xai_key = std::env::var("XAI_API_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .is_some();
    let has_supergrok = roder_supergrok_auth::status()
        .await
        .ok()
        .flatten()
        .is_some();

    assert!(
        has_xai_key || has_supergrok,
        "live xAI smoke tests require XAI_API_KEY or `roder auth login supergrok`"
    );
}
