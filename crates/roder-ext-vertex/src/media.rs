use roder_api::transcript::{InputImage, UserMessage};
use serde_json::{Value, json};

pub(crate) fn vertex_user_message_parts(message: &UserMessage) -> anyhow::Result<Vec<Value>> {
    let mut parts = Vec::new();
    if !message.text.is_empty() {
        parts.push(json!({ "text": message.text }));
    }
    for image in &message.images {
        parts.push(vertex_image_part(image)?);
    }
    if parts.is_empty() {
        parts.push(json!({ "text": "" }));
    }
    Ok(parts)
}

fn vertex_image_part(image: &InputImage) -> anyhow::Result<Value> {
    let (mime_type, data) = parse_base64_data_url(&image.image_url)
        .ok_or_else(|| anyhow::anyhow!("Vertex AI image input requires a base64 data URL image"))?;
    anyhow::ensure!(
        mime_type.starts_with("image/"),
        "Vertex AI image input requires an image MIME type, got {mime_type}"
    );
    anyhow::ensure!(!data.is_empty(), "Vertex AI image input data is empty");
    Ok(json!({
        "inline_data": {
            "mime_type": mime_type,
            "data": data,
        }
    }))
}

fn parse_base64_data_url(image_url: &str) -> Option<(&str, &str)> {
    let body = image_url.strip_prefix("data:")?;
    let (metadata, data) = body.split_once(',')?;
    let mut metadata_parts = metadata.split(';');
    let mime_type = metadata_parts.next().filter(|mime| !mime.is_empty())?;
    metadata_parts
        .any(|part| part.eq_ignore_ascii_case("base64"))
        .then_some((mime_type, data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_data_url_images_to_inline_data() {
        let parts = vertex_user_message_parts(&UserMessage::with_images(
            "what is shown?",
            vec![InputImage {
                image_url: "data:image/png;base64,YWJj".to_string(),
            }],
        ))
        .unwrap();

        assert_eq!(parts[0]["text"], "what is shown?");
        assert_eq!(parts[1]["inline_data"]["mime_type"], "image/png");
        assert_eq!(parts[1]["inline_data"]["data"], "YWJj");
    }

    #[test]
    fn rejects_non_data_url_images() {
        let err = vertex_user_message_parts(&UserMessage::with_images(
            "what is shown?",
            vec![InputImage {
                image_url: "https://example.com/image.png".to_string(),
            }],
        ))
        .unwrap_err();

        assert!(err.to_string().contains("base64 data URL image"), "{err}");
    }
}
