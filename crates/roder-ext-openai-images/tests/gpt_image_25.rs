use roder_api::media::{MediaGenerationRequest, MediaGeneratorProvider, MediaImageInput};
use roder_ext_openai_images::{OpenAiImagesConfig, OpenAiImagesProvider};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

async fn provider() -> (OpenAiImagesProvider, JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let request = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        let mut buffer = [0; 16384];
        loop {
            let read = stream.read(&mut buffer).await.unwrap();
            assert_ne!(read, 0);
            bytes.extend_from_slice(&buffer[..read]);
            let text = String::from_utf8_lossy(&bytes);
            if let Some(end) = text.find("\r\n\r\n") {
                let length = text[..end]
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|value| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap();
                if bytes.len() >= end + 4 + length {
                    break;
                }
            }
        }
        let body = r#"{"data":[{"b64_json":"aW1hZ2U="}]}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).await.unwrap();
        stream.shutdown().await.unwrap();
        String::from_utf8(bytes).unwrap()
    });
    (
        OpenAiImagesProvider::new(
            OpenAiImagesConfig::new(Some("test-image-key".to_string())).with_base_url(base_url),
        ),
        request,
    )
}

#[test]
fn gpt_image_25_models_are_advertised_with_flare_as_default() {
    let descriptor = OpenAiImagesProvider::new(OpenAiImagesConfig::new(None)).descriptor();
    assert_eq!(
        descriptor.default_model.as_deref(),
        Some("gpt-image-2.5-flare")
    );
    for id in ["gpt-image-2.5-flare", "gpt-image-2.5-sunburst"] {
        let model = descriptor
            .image_models
            .iter()
            .find(|model| model.id == id)
            .unwrap();
        assert_eq!(model.provider, "openai");
        assert_eq!(model.is_default, id == "gpt-image-2.5-flare");
        assert!(!model.legacy);
        assert!(model.supports_edit);
        assert!(model.supports_multiple_outputs);
        assert!(model.supports_transparent_background);
    }
}

#[tokio::test]
async fn omitted_model_uses_flare_on_the_wire_and_in_result_metadata() {
    let (provider, captured) = provider().await;
    let batch = provider
        .generate_image(MediaGenerationRequest {
            prompt: "a tiny transparent character".to_string(),
            ..MediaGenerationRequest::default()
        })
        .await
        .unwrap();
    let raw = captured.await.unwrap();
    let body: serde_json::Value =
        serde_json::from_str(raw.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(body["model"], "gpt-image-2.5-flare");
    assert_eq!(batch.model, "gpt-image-2.5-flare");
}

#[tokio::test]
async fn both_gpt_image_25_models_preserve_all_documented_quality_levels() {
    for model in ["gpt-image-2.5-flare", "gpt-image-2.5-sunburst"] {
        for quality in ["auto", "low", "medium", "high", "xhigh", "max"] {
            let (provider, captured) = provider().await;
            let batch = provider
                .generate_image(MediaGenerationRequest {
                    model: Some(model.to_string()),
                    prompt: "a tiny transparent character".to_string(),
                    quality: Some(quality.to_string()),
                    size: Some("1024x1024".to_string()),
                    background: Some("transparent".to_string()),
                    output_format: Some("png".to_string()),
                    ..MediaGenerationRequest::default()
                })
                .await
                .unwrap();
            let raw = captured.await.unwrap();
            assert!(raw.starts_with("POST /images/generations "));
            let body: serde_json::Value =
                serde_json::from_str(raw.split_once("\r\n\r\n").unwrap().1).unwrap();
            assert_eq!(body["model"], model);
            assert_eq!(body["quality"], quality);
            assert_eq!(body["background"], "transparent");
            assert_eq!(batch.model, model);
            assert_eq!(batch.images[0].bytes_base64, "aW1hZ2U=");
        }
    }
}

#[tokio::test]
async fn gpt_image_25_reference_edits_keep_the_requested_model_and_quality() {
    for model in ["gpt-image-2.5-flare", "gpt-image-2.5-sunburst"] {
        let (provider, captured) = provider().await;
        let batch = provider
            .generate_image(MediaGenerationRequest {
                model: Some(model.to_string()),
                prompt: "give the character an orange scarf".to_string(),
                quality: Some("max".to_string()),
                input_images: vec![MediaImageInput {
                    bytes_base64: "aW5wdXQ=".to_string(),
                    mime_type: "image/png".to_string(),
                }],
                ..MediaGenerationRequest::default()
            })
            .await
            .unwrap();
        let raw = captured.await.unwrap();
        assert!(raw.starts_with("POST /images/edits "));
        assert!(raw.contains(&format!("name=\"model\"\r\n\r\n{model}\r\n")));
        assert!(raw.contains("name=\"quality\"\r\n\r\nmax\r\n"));
        assert!(raw.contains("name=\"image[]\"; filename=\"input-0.png\""));
        assert_eq!(batch.model, model);
    }
}
