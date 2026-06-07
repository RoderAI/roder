use roder_api::embeddings::{EmbeddingInputType, EmbeddingProvider, EmbeddingRequest};
use roder_ext_zeroentropy_embeddings::{
    DEFAULT_MODEL, ZeroEntropyEmbeddingProvider, ZeroEntropyEmbeddingsConfig,
};

#[tokio::test]
#[ignore = "requires live ZeroEntropy API credentials"]
async fn live_zeroentropy_embedding_smoke() {
    if std::env::var("RODER_ZEROENTROPY_EMBEDDINGS_LIVE")
        .ok()
        .as_deref()
        != Some("1")
    {
        eprintln!(
            "skipping live ZeroEntropy embeddings smoke; set RODER_ZEROENTROPY_EMBEDDINGS_LIVE=1"
        );
        return;
    }
    let config = ZeroEntropyEmbeddingsConfig::from_env();
    assert!(
        config
            .api_key
            .as_deref()
            .is_some_and(|key| !key.trim().is_empty()),
        "live ZeroEntropy embeddings smoke requires RODER_ZEROENTROPY_API_KEY or ZEROENTROPY_API_KEY"
    );

    let provider = ZeroEntropyEmbeddingProvider::new(config);
    let response = provider
        .embed(EmbeddingRequest {
            model: DEFAULT_MODEL.to_string(),
            inputs: vec!["hello from roder live embedding check".to_string()],
            input_type: EmbeddingInputType::Document,
            dimensions: Some(80),
        })
        .await
        .unwrap();

    assert_eq!(response.provider_id, "zeroentropy");
    assert_eq!(response.model, DEFAULT_MODEL);
    assert_eq!(response.embeddings.len(), 1);
    assert_eq!(response.embeddings[0].index, 0);
    assert_eq!(response.embeddings[0].values.len(), 80);
}
