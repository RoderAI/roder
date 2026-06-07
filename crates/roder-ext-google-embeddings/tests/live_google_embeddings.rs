use roder_api::embeddings::{EmbeddingInputType, EmbeddingProvider, EmbeddingRequest};
use roder_ext_google_embeddings::{DEFAULT_MODEL, GoogleEmbeddingProvider, GoogleEmbeddingsConfig};

#[tokio::test]
#[ignore = "requires RODER_GOOGLE_EMBEDDINGS_LIVE=1 and a Gemini/Google API key"]
async fn live_gemini_embedding_2_with_api_key() {
    if std::env::var("RODER_GOOGLE_EMBEDDINGS_LIVE")
        .ok()
        .as_deref()
        != Some("1")
    {
        eprintln!("set RODER_GOOGLE_EMBEDDINGS_LIVE=1 to run live Google embeddings smoke");
        return;
    }
    let config = GoogleEmbeddingsConfig::from_env();
    assert!(
        config
            .api_key
            .as_ref()
            .is_some_and(|key| !key.trim().is_empty()),
        "live Google embeddings smoke requires RODER_GOOGLE_EMBEDDINGS_API_KEY, GEMINI_API_TOKEN, GEMINI_API_KEY, GOOGLE_API_KEY, GOOGLE_GENAI_API_KEY, or GOOGLE_AI_API_KEY"
    );

    let provider = GoogleEmbeddingProvider::new(config);
    let response = provider
        .embed(EmbeddingRequest {
            model: DEFAULT_MODEL.to_string(),
            inputs: vec!["hello from roder live embedding check".to_string()],
            input_type: EmbeddingInputType::Document,
            dimensions: Some(8),
        })
        .await
        .unwrap();

    assert_eq!(response.provider_id, "google");
    assert_eq!(response.model, DEFAULT_MODEL);
    assert_eq!(response.embeddings.len(), 1);
    assert_eq!(response.embeddings[0].index, 0);
    assert_eq!(response.embeddings[0].values.len(), 8);
}
