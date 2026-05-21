#[tokio::test]
#[ignore = "requires RODER_OPENAI_LIVE=1 and OPENAI_API_KEY"]
async fn live_openai_tools_smoke_is_explicitly_opt_in() {
    if std::env::var("RODER_OPENAI_LIVE").ok().as_deref() != Some("1") {
        eprintln!("set RODER_OPENAI_LIVE=1 to run live OpenAI tool smoke tests");
        return;
    }

    let has_key = std::env::var("OPENAI_API_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .is_some();

    assert!(
        has_key,
        "live OpenAI tool smoke tests require OPENAI_API_KEY"
    );
}
