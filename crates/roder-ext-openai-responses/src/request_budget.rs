use super::*;

pub(super) struct RequestPayload {
    pub bytes: bytes::Bytes,
    pub metadata: Value,
}

/// Serialize the request exactly as built. Roder never sheds or rewrites input
/// to fit a client-side size budget: the provider reports an oversized request
/// (HTTP 413 or a context-window error) and the runtime compacts in response.
/// Unchanged history also keeps the provider prompt cache valid.
pub(super) fn prepare_request_payload(body: &Value) -> anyhow::Result<RequestPayload> {
    let bytes = serde_json::to_vec(body)?;
    let (images, image_bytes) = image_sizes(body);
    Ok(RequestPayload {
        metadata: json!({"type":"request_budget","request_bytes":bytes.len(),"image_count":images,
            "image_bytes":image_bytes}),
        bytes: bytes.into(),
    })
}

fn image_sizes(value: &Value) -> (usize, usize) {
    match value {
        Value::Object(fields)
            if fields
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| matches!(kind, "input_image" | "computer_screenshot")) =>
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
    fn large_requests_are_sent_unchanged_for_the_provider_to_judge() {
        let image = |id| json!({"type":"function_call_output","call_id":id,"output":[{"type":"input_image","image_url":format!("data:image/png;base64,{}","x".repeat(20 * 1024 * 1024))}]});
        let body = json!({"input":[image("old"),image("new")]});
        let payload = prepare_request_payload(&body).unwrap();
        assert_eq!(payload.bytes, serde_json::to_vec(&body).unwrap());
        assert_eq!(payload.metadata["image_count"], 2);
    }
}
