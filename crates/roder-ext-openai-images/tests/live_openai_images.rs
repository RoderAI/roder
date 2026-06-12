//! Opt-in live smoke for OpenAI GPT Image generation.
//!
//! Requires `RODER_OPENAI_IMAGE_LIVE=1` and `OPENAI_API_KEY`. Generates one
//! tiny low-quality image, writes it to a temp dir, and deletes it. Never
//! runs in normal test or CI invocations.

use base64::Engine;
use roder_api::media::{MediaGenerationRequest, MediaGeneratorProvider};
use roder_ext_openai_images::{OpenAiImagesConfig, OpenAiImagesProvider};

#[tokio::test]
#[ignore = "requires RODER_OPENAI_IMAGE_LIVE=1 and OPENAI_API_KEY"]
async fn live_openai_image_generation_smoke_is_explicitly_opt_in() {
    if std::env::var("RODER_OPENAI_IMAGE_LIVE").ok().as_deref() != Some("1") {
        eprintln!("set RODER_OPENAI_IMAGE_LIVE=1 to run the live OpenAI image smoke test");
        return;
    }
    let api_key = std::env::var("OPENAI_API_KEY")
        .expect("OPENAI_API_KEY must be set for the live OpenAI image smoke test");

    let provider = OpenAiImagesProvider::new(OpenAiImagesConfig::new(Some(api_key)));
    let batch = provider
        .generate_image(MediaGenerationRequest {
            prompt: "A single small blue dot on a white background".to_string(),
            model: Some("gpt-image-2".to_string()),
            size: Some("1024x1024".to_string()),
            quality: Some("low".to_string()),
            ..MediaGenerationRequest::default()
        })
        .await
        .expect("live OpenAI image generation succeeds");

    assert_eq!(batch.images.len(), 1);
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&batch.images[0].bytes_base64)
        .expect("live output is valid base64");
    assert!(!bytes.is_empty());

    let dir = std::env::temp_dir().join(format!("roder-openai-image-live-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("smoke.png");
    std::fs::write(&path, &bytes).unwrap();
    eprintln!("live OpenAI image written to {} ({} bytes)", path.display(), bytes.len());
    std::fs::remove_dir_all(&dir).unwrap();
}
