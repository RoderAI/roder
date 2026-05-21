#[tokio::test]
#[ignore = "requires RODER_OPENCODE_LIVE=1 and an OpenCode API key"]
async fn live_opencode_tools_smoke_is_explicitly_opt_in() {
    if std::env::var("RODER_OPENCODE_LIVE").ok().as_deref() != Some("1") {
        eprintln!("set RODER_OPENCODE_LIVE=1 to run live OpenCode tool smoke tests");
        return;
    }

    let has_key = [
        "OPENCODE_API_KEY",
        "OPENCODE_ZEN_API_KEY",
        "RODER_OPENCODE_API_KEY",
        "OPENCODE_GO_API_KEY",
        "RODER_OPENCODE_GO_API_KEY",
    ]
    .iter()
    .any(|name| {
        std::env::var(name)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .is_some()
    });

    assert!(has_key, "live OpenCode tool smoke tests require an API key");
}
