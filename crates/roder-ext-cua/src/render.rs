//! Keep grounding handles inline when macOS returns a large application tree.
use serde_json::{Value, json};

pub(crate) fn result_text(data: &Value) -> String {
    let mut text = data.clone();
    text.as_object_mut()
        .unwrap()
        .remove(roder_api::transcript::VIEW_IMAGE_DISPLAY_KEY);
    for field in ["observation", "after_action"] {
        if let Some(observation) = text.get_mut(field).and_then(Value::as_object_mut) {
            // Structured rows carry the same actions/geometry without the
            // repeated global menu markdown and absolute element rectangles.
            observation.remove("_note");
            if let Some(tree) = observation.get("tree_markdown").and_then(Value::as_str) {
                let window = tree.split("\n- ").next().unwrap_or(tree);
                let mut excerpt: String = window.chars().take(4000).collect();
                if window.chars().count() > 4000 {
                    excerpt.push_str("\n[tree excerpt capped; query a narrower label]");
                }
                observation.insert("tree_markdown".into(), json!(excerpt));
            }
            if let Some(elements) = observation
                .get_mut("elements")
                .and_then(Value::as_array_mut)
            {
                for element in elements {
                    if let Some(object) = element.as_object_mut() {
                        object.remove("frame");
                    }
                }
            }
        }
    }
    cap_strings(&mut text);
    // The core spills outputs over 20k characters into an artifact. Keep the
    // capture and element handles in valid JSON the model can immediately use.
    let mut omitted = 0;
    while text.to_string().chars().count() > 18_000 {
        let rows = if text["after_action"]["elements"].is_array() {
            "after_action"
        } else {
            "observation"
        };
        let Some(elements) = text[rows]["elements"].as_array_mut() else {
            break;
        };
        if elements.is_empty() {
            break;
        }
        let remove = (elements.len() / 4).max(1);
        elements.truncate(elements.len() - remove);
        omitted += remove;
        text[rows]["elements_omitted_from_text"] = json!(omitted);
    }
    text.to_string()
}

fn cap_strings(value: &mut Value) {
    match value {
        Value::String(text) if text.chars().count() > 2000 => {
            *text = format!(
                "{} [excerpt capped; full value in structured observation]",
                text.chars().take(2000).collect::<String>()
            );
        }
        Value::Array(values) => values.iter_mut().for_each(cap_strings),
        Value::Object(values) => values.values_mut().for_each(cap_strings),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn large_tree_keeps_capture_and_action_handles_in_valid_inline_json() {
        let mut data = json!({"untrusted":true,"__view_image":{"image_url":"secret-pixels"},"after_action":{
            "capture_id":"current","snapshot_id":"bound","tree_markdown":"- window\n- global menu",
            "elements":(0..500).map(|i|json!({"element_token":format!("bound:{i}"),"label":"x".repeat(200),"screenshot_frame":{"x":1,"y":1,"w":1,"h":1},"frame":{"x":100,"y":100}})).collect::<Vec<_>>()}});
        let before = data.clone();
        let text = result_text(&data);
        assert!(text.chars().count() <= 18_000);
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["after_action"]["capture_id"], "current");
        assert_eq!(
            value["after_action"]["elements"][0]["element_token"],
            "bound:0"
        );
        assert!(
            value["after_action"]["elements_omitted_from_text"]
                .as_u64()
                .unwrap()
                > 0
        );
        assert!(value.get("__view_image").is_none());
        assert_eq!(data, before);
        data["observation"] = json!({"code":"refused","summary":"No input was sent"});
        assert!(result_text(&data).contains("No input was sent"));
    }
}
