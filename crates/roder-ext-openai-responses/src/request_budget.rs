use super::*;

const DEFAULT_REQUEST_BYTES: usize = 15 * 1024 * 1024;

pub(super) fn request_byte_limit() -> usize {
    std::env::var("RODER_RESPONSES_MAX_REQUEST_BYTES")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|bytes| *bytes > 0)
        .unwrap_or(DEFAULT_REQUEST_BYTES)
}

pub(super) struct RequestPayload {
    pub bytes: bytes::Bytes,
    pub metadata: Value,
}

/// Measure the actual serialized wire payload, including schemas, opaque
/// provider items and escaped UTF-8. Shed only previously viewed tool images;
/// the newest image and all canonical transcript items remain available.
pub(super) fn prepare_request_payload(
    body: &Value,
    limit: usize,
) -> anyhow::Result<RequestPayload> {
    let mut bytes = serde_json::to_vec(body)?;
    let (mut images, mut image_bytes) = image_sizes(body);
    let mut shed_tool_images = 0;
    if bytes.len() > limit {
        let mut compact = body.clone();
        if let Some(input) = compact.get("input").and_then(Value::as_array) {
            let image_outputs: Vec<usize> = input
                .iter()
                .enumerate()
                .filter_map(|(index, item)| {
                    (item["type"] == "function_call_output"
                        && item
                            .get("output")
                            .and_then(Value::as_array)
                            .is_some_and(|blocks| {
                                blocks.iter().any(|block| block["type"] == "input_image")
                            }))
                    .then_some(index)
                })
                .collect();
            for index in image_outputs
                .iter()
                .take(image_outputs.len().saturating_sub(1))
            {
                let (removed_images, removed_bytes) =
                    image_sizes(&compact["input"][*index]["output"]);
                images -= removed_images;
                image_bytes -= removed_bytes;
                let blocks = compact["input"][*index]["output"]
                    .as_array_mut()
                    .expect("image output blocks");
                blocks.retain(|block| block["type"] != "input_image");
                blocks.push(json!({"type":"input_text", "text":
                    "Previously viewed image omitted to fit the byte budget. Capture a fresh screenshot if needed."}));
                shed_tool_images += 1;
                bytes = serde_json::to_vec(&compact)?;
                if bytes.len() <= limit {
                    break;
                }
            }
        }
    }
    if bytes.len() > limit {
        return Err(ProviderFailure::new(ProviderFailureKind::RequestTooLarge,
            format!("Responses request is {} bytes, exceeding the configured {limit}-byte budget. Reduce attached images or tool definitions; RODER_RESPONSES_MAX_REQUEST_BYTES sets this client limit.",bytes.len())).into());
    }
    Ok(RequestPayload {
        metadata: json!({"type":"request_budget","request_bytes":bytes.len(),"image_count":images,
            "image_bytes":image_bytes,"shed_tool_images":shed_tool_images,"limit_bytes":limit}),
        bytes: bytes.into(),
    })
}

fn image_sizes(value: &Value) -> (usize, usize) {
    match value {
        Value::Object(fields)
            if fields.get("type").and_then(Value::as_str) == Some("input_image") =>
        {
            (
                1,
                fields
                    .get("image_url")
                    .and_then(Value::as_str)
                    .map_or(0, str::len),
            )
        }
        Value::Object(fields) => fields
            .values()
            .map(image_sizes)
            .fold((0, 0), |(count, size), (c, s)| (count + c, size + s)),
        Value::Array(items) => items
            .iter()
            .map(image_sizes)
            .fold((0, 0), |(count, size), (c, s)| (count + c, size + s)),
        _ => (0, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shedding_old_screenshots_preserves_page_observations_and_newest_image() {
        let image = |id, observation| {
            json!({"type":"function_call_output", "call_id":id,
            "output":[{"type":"input_text", "text":observation},
                {"type":"input_image", "image_url":format!("data:image/png;base64,{}", "A".repeat(1000)), "detail":"original"}]})
        };
        let body = json!({"input":[image("old", "Cart total: 12.00"), image("new", "Order still pending")]});
        let payload =
            prepare_request_payload(&body, serde_json::to_vec(&body).unwrap().len() - 500).unwrap();
        let wire: Value = serde_json::from_slice(&payload.bytes).unwrap();
        assert_eq!(wire["input"][0]["call_id"], "old");
        assert_eq!(wire["input"][0]["output"][0]["text"], "Cart total: 12.00");
        assert!(
            wire["input"][0]["output"]
                .as_array()
                .unwrap()
                .iter()
                .all(|block| block["type"] != "input_image")
        );
        assert_eq!(wire["input"][1]["output"], body["input"][1]["output"]);
    }

    #[test]
    fn budget_counts_serialized_schemas_and_opaque_state() {
        let body = json!({"input":[{"type":"reasoning","encrypted_content":"x".repeat(1000)}],"tools":[{"description":"é".repeat(1000)}]});
        let limit = serde_json::to_vec(&body).unwrap().len();
        assert_eq!(
            prepare_request_payload(&body, limit).unwrap().bytes.len(),
            limit
        );
        assert_eq!(
            prepare_request_payload(&body, limit - 1)
                .err()
                .unwrap()
                .downcast_ref::<ProviderFailure>()
                .unwrap()
                .kind,
            ProviderFailureKind::RequestTooLarge
        );
    }
    #[test]
    fn budget_sheds_only_older_tool_images_without_mutating_history() {
        let image = |id| json!({"type":"function_call_output","call_id":id,"output":[{"type":"input_image","image_url":format!("data:image/png;base64,{}","x".repeat(1000))}]});
        let body = json!({"input":[image("old"),image("new")]});
        let payload = prepare_request_payload(&body, 1500).unwrap();
        let wire: Value = serde_json::from_slice(&payload.bytes).unwrap();
        assert_eq!(wire["input"][0]["output"][0]["type"], "input_text");
        assert!(wire["input"][1]["output"].is_array());
        assert!(body["input"][0]["output"].is_array());
        assert_eq!(payload.metadata["image_count"], 1);
        assert_eq!(payload.metadata["shed_tool_images"], 1);
    }
}
