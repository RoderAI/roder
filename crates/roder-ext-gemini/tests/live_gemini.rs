#[tokio::test]
#[ignore = "requires RODER_GEMINI_LIVE=1 and GEMINI_API_KEY or GEMINI_API_TOKEN"]
async fn live_gemini_tools_smoke_is_explicitly_opt_in() {
    if std::env::var("RODER_GEMINI_LIVE").ok().as_deref() != Some("1") {
        eprintln!("set RODER_GEMINI_LIVE=1 to run live Gemini tool smoke tests");
        return;
    }

    let has_key = ["GEMINI_API_KEY", "GEMINI_API_TOKEN"].iter().any(|name| {
        std::env::var(name)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .is_some()
    });

    assert!(
        has_key,
        "live Gemini tool smoke tests require GEMINI_API_KEY or GEMINI_API_TOKEN"
    );
}
