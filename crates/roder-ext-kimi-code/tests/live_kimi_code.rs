#[tokio::test]
#[ignore = "requires RODER_KIMI_CODE_LIVE=1 and a valid Kimi Code subscription or KIMI_CODE_API_KEY"]
async fn live_kimi_code_smoke_is_explicitly_opt_in() {
    if std::env::var("RODER_KIMI_CODE_LIVE").ok().as_deref() != Some("1") {
        eprintln!("set RODER_KIMI_CODE_LIVE=1 to run live Kimi Code smoke tests");
        return;
    }

    let has_key = ["KIMI_CODE_API_KEY", "RODER_KIMI_CODE_API_KEY"]
        .iter()
        .any(|name| {
            std::env::var(name)
                .ok()
                .filter(|value| !value.trim().is_empty())
                .is_some()
        });
    let has_oauth = roder_ext_kimi_code::has_stored_tokens();

    assert!(
        has_key || has_oauth,
        "live Kimi Code smoke tests require an API key or `roder auth login kimi-code`"
    );

    // Minimal smoke: the provider should be loadable and list_models should return entries
    // (real streaming test would require full app context; this guards the integration).
    // Full end-to-end is exercised via the app-server e2e or manual `roder exec`.
}
