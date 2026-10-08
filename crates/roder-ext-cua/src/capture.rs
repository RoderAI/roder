use base64::Engine;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub(crate) struct Capture {
    pub image: Value,
    pub id: String,
    pub snapshot: Option<String>,
    pub width: u32,
    pub height: u32,
}

pub(crate) fn take_capture(observation: &mut Value, session: &str) -> anyhow::Result<Capture> {
    let encoded = observation
        .as_object_mut()
        .and_then(|object| object.remove("screenshot_png_b64"))
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or_else(|| anyhow::anyhow!("Cua capture returned no PNG"))?;
    anyhow::ensure!(encoded.len() <= 12 * 1024 * 1024, "Cua PNG too large");
    let mime = observation
        .as_object_mut()
        .and_then(|object| object.remove("screenshot_mime_type"));
    anyhow::ensure!(
        mime.as_ref().and_then(Value::as_str).unwrap_or("image/png") == "image/png",
        "unexpected Cua screenshot MIME type"
    );
    let bytes = base64::engine::general_purpose::STANDARD.decode(&encoded)?;
    anyhow::ensure!(
        bytes.len() >= 33
            && bytes.len() <= 8 * 1024 * 1024
            && bytes.starts_with(b"\x89PNG\r\n\x1a\n")
            && &bytes[12..16] == b"IHDR",
        "invalid Cua PNG"
    );
    let width = u32::from_be_bytes(bytes[16..20].try_into()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into()?);
    anyhow::ensure!(
        width > 0
            && height > 0
            && width <= 8192
            && height <= 8192
            && u64::from(width) * u64::from(height) <= 16_777_216,
        "invalid Cua PNG dimensions"
    );
    anyhow::ensure!(
        observation["screenshot_width"].as_u64() == Some(u64::from(width))
            && observation["screenshot_height"].as_u64() == Some(u64::from(height)),
        "Cua PNG metadata does not match its pixels"
    );
    let mut decoder = png::Decoder::new(std::io::Cursor::new(&bytes));
    decoder.set_limits(png::Limits {
        bytes: 64 * 1024 * 1024,
    });
    let mut reader = decoder.read_info()?;
    let size = reader.output_buffer_size();
    anyhow::ensure!(size <= 64 * 1024 * 1024, "Cua decoded PNG exceeds 64 MiB");
    let mut decoded = vec![0; size];
    reader.next_frame(&mut decoded)?;
    let id = observation["capture_id"]
        .as_str()
        .filter(|id| !id.is_empty() && id.len() <= 256)
        .ok_or_else(|| anyhow::anyhow!("missing capture_id"))?
        .to_owned();
    let snapshot = bind_snapshot(observation, session, &id);
    Ok(Capture {
        image: json!({"image_url":format!("data:image/png;base64,{encoded}"),"detail":"original"}),
        id,
        snapshot,
        width,
        height,
    })
}

/// Driver snapshot counters can repeat on another desktop or after restart.
/// Bind the public token to this thread/runner and globally unique capture.
fn bind_snapshot(observation: &mut Value, session: &str, capture: &str) -> Option<String> {
    let native = observation["snapshot_id"].as_str()?.to_owned();
    let digest = Sha256::digest(serde_json::to_vec(&(session, capture, &native)).ok()?);
    let nonce = digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let bound = format!("roder-{nonce}/{native}");
    observation["snapshot_id"] = json!(bound);
    if let Some(elements) = observation["elements"].as_array_mut() {
        for element in elements {
            if let Some(token) = element["element_token"].as_str().map(str::to_owned)
                && let Some(row) = token.strip_prefix(&format!("{native}:"))
            {
                element["element_token"] = json!(format!("{bound}:{row}"));
            }
        }
    }
    if let Some(tree) = observation["tree_markdown"].as_str() {
        observation["tree_markdown"] =
            json!(tree.replace(&format!("{native}:"), &format!("{bound}:")));
    }
    Some(bound)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn observation() -> Value {
        json!({"capture_id":"capture-one","screenshot_width":2,"screenshot_height":2,
            "screenshot_png_b64":include_str!("../tests/capture.b64")})
    }
    #[test]
    fn rejects_metadata_mismatch_and_corrupt_pixels() {
        let mut valid = observation();
        assert!(take_capture(&mut valid, "thread-one").is_ok());
        assert!(valid.get("screenshot_png_b64").is_none());
        let mut wrong = observation();
        wrong["screenshot_width"] = json!(3);
        assert!(take_capture(&mut wrong, "thread-one").is_err());
        let mut wrong = observation();
        wrong["screenshot_mime_type"] = json!("image/jpeg");
        assert!(take_capture(&mut wrong, "thread-one").is_err());
        let mut wrong = observation();
        let mut bytes = base64::engine::general_purpose::STANDARD
            .decode(wrong["screenshot_png_b64"].as_str().unwrap())
            .unwrap();
        bytes.truncate(33);
        wrong["screenshot_png_b64"] =
            json!(base64::engine::general_purpose::STANDARD.encode(bytes));
        assert!(take_capture(&mut wrong, "thread-one").is_err());
    }

    #[test]
    fn repeated_driver_counters_are_distinct_across_runners_and_restarts() {
        let source = json!({"snapshot_id":"s00000001","elements":[{"element_token":"s00000001:0"}],"tree_markdown":"[s00000001:0] field"});
        let mut a = source.clone();
        let mut b = source.clone();
        let mut restarted = source;
        let first = bind_snapshot(&mut a, "runner-one", "boot-one-capture").unwrap();
        assert_ne!(
            first,
            bind_snapshot(&mut b, "runner-two", "boot-one-capture").unwrap()
        );
        assert_ne!(
            first,
            bind_snapshot(&mut restarted, "runner-one", "boot-two-capture").unwrap()
        );
        assert_eq!(a["elements"][0]["element_token"], format!("{first}:0"));
        assert_eq!(a["tree_markdown"], format!("[{first}:0] field"));
    }
}
