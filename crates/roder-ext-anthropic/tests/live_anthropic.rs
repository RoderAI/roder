#[tokio::test]
#[ignore = "requires RODER_ANTHROPIC_LIVE=1 and ANTHROPIC_API_KEY"]
async fn live_anthropic_tools_smoke_is_explicitly_opt_in() {
    if std::env::var("RODER_ANTHROPIC_LIVE").ok().as_deref() != Some("1") {
        eprintln!("set RODER_ANTHROPIC_LIVE=1 to run live Anthropic tool smoke tests");
        return;
    }

    let has_key = std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .is_some();

    assert!(
        has_key,
        "live Anthropic tool smoke tests require ANTHROPIC_API_KEY"
    );
}
