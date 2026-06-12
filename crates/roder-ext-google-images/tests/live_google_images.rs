//! Opt-in live smoke for Google Gemini (Nano Banana) image generation.
//!
//! Requires `RODER_GEMINI_IMAGE_LIVE=1` plus `GEMINI_API_KEY` or
//! `GEMINI_API_TOKEN`. Generates one image, writes it to a temp dir, and
//! deletes it. Never runs in normal test or CI invocations.

use base64::Engine;
use roder_api::media::{MediaGenerationRequest, MediaGeneratorProvider};
use roder_ext_google_images::{GoogleImagesConfig, GoogleImagesProvider};

#[tokio::test]
#[ignore = "requires RODER_GEMINI_IMAGE_LIVE=1 and GEMINI_API_KEY or GEMINI_API_TOKEN"]
async fn live_gemini_image_generation_smoke_is_explicitly_opt_in() {
    if std::env::var("RODER_GEMINI_IMAGE_LIVE").ok().as_deref() != Some("1") {
        eprintln!("set RODER_GEMINI_IMAGE_LIVE=1 to run the live Gemini image smoke test");
        return;
    }
    let api_key = std::env::var("GEMINI_API_KEY")
        .or_else(|_| std::env::var("GEMINI_API_TOKEN"))
        .expect(
            "GEMINI_API_KEY or GEMINI_API_TOKEN must be set for the live Gemini image smoke test",
        );

    let provider = GoogleImagesProvider::new(GoogleImagesConfig::new(Some(api_key)));
    let batch = provider
        .generate_image(MediaGenerationRequest {
            prompt: "A single small green dot on a white background".to_string(),
            model: Some("gemini-2.5-flash-image".to_string()),
            ..MediaGenerationRequest::default()
        })
        .await
        .expect("live Gemini image generation succeeds");

    assert!(!batch.images.is_empty());
    assert_eq!(batch.images[0].watermark.as_deref(), Some("synthid"));
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&batch.images[0].bytes_base64)
        .expect("live output is valid base64");
    assert!(!bytes.is_empty());

    let dir = std::env::temp_dir().join(format!("roder-gemini-image-live-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("smoke.png");
    std::fs::write(&path, &bytes).unwrap();
    eprintln!(
        "live Gemini image written to {} ({} bytes)",
        path.display(),
        bytes.len()
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
