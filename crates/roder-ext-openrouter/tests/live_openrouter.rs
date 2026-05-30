#[tokio::test]
#[ignore = "requires RODER_OPENROUTER_LIVE=1 and OPENROUTER_API_KEY"]
async fn live_openrouter_grok_build_minimal_responses_smoke() {
    if std::env::var("RODER_OPENROUTER_LIVE").ok().as_deref() != Some("1") {
        eprintln!("skipping live OpenRouter smoke; set RODER_OPENROUTER_LIVE=1");
        return;
    }
    let Some(api_key) = std::env::var("OPENROUTER_API_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
    else {
        eprintln!("skipping live OpenRouter smoke; OPENROUTER_API_KEY is not set");
        return;
    };
    let model = std::env::var("OPENROUTER_LIVE_MODEL")
        .unwrap_or_else(|_| "x-ai/grok-build-0.1".to_string());
    let base_url = std::env::var("OPENROUTER_BASE_URL")
        .unwrap_or_else(|_| "https://openrouter.ai/api/v1".to_string())
        .trim_end_matches('/')
        .to_string();

    let response = reqwest::Client::new()
        .post(format!("{base_url}/responses"))
        .bearer_auth(api_key)
        .json(&serde_json::json!({
            "model": model,
            "input": "Reply with exactly: ok",
            "max_output_tokens": 8,
            "stream": false
        }))
        .send()
        .await
        .expect("live OpenRouter request should be sent");

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    assert!(
        status.is_success(),
        "OpenRouter live smoke failed with status {status}; body excerpt: {}",
        redact_body(&body)
    );
}

fn redact_body(body: &str) -> String {
    let mut excerpt = body.chars().take(300).collect::<String>();
    for key in ["OPENROUTER_API_KEY", "Authorization", "Bearer", "sk-or-"] {
        excerpt = excerpt.replace(key, "[redacted]");
    }
    excerpt
}
