//! Opt-in image input on the wire: the real hosted client and transport
//! against a local server.

use image::ImageFormat;

use super::images::tests::{data_url, dimensions};
use super::*;
use crate::http::tests::{MockServer, Reply, fast_policy};

const DONE: &str = r#"{"answers":{"operation":{"choice":"DONE","confidence":1.0,"probabilities":{"DONE":1.0,"BLOCKED":0.0}}},"model":"jev-1.14.0"}"#;

fn client(server: &MockServer, images: bool) -> JevTypeSafeDecisionClient {
    JevTypeSafeDecisionClient::with_transport(
        "jev-preview",
        Arc::new(TypeSafeHttpTransport::new(
            &server.url,
            "sk-typesafe",
            fast_policy(),
        )),
    )
    .with_images(images)
}

fn page(screenshot: Option<String>) -> Value {
    let mut page = json!({"url":"https://a.test/","title":"A","text":"Thanks","actions":[]});
    if let Some(screenshot) = screenshot {
        page["_screenshot"] = json!(screenshot);
    }
    page
}

fn images_header(head: &str) -> Option<String> {
    head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case(images::IMAGES_HEADER)
            .then(|| value.trim().to_string())
    })
}

/// The request with today's date, which differs between runs only by day.
fn without_date(mut body: Value) -> Value {
    body["state"].as_object_mut().unwrap().remove("date");
    body
}

#[tokio::test]
async fn an_enabled_client_sends_one_fitted_asset_referenced_from_state_with_the_header() {
    let server = MockServer::start(vec![Reply::ok(DONE)]).await;
    let client = client(&server, true);
    assert!(client.uses_images());
    let screenshot = data_url(ImageFormat::Png, 2240, 1560);
    let decision = client
        .choose(&page(Some(screenshot)), "Finish", &[])
        .await
        .unwrap();
    assert_eq!(decision.operation, "DONE");

    assert_eq!(images_header(&server.heads()[0]).as_deref(), Some("true"));
    let body = &server.requests()[0].1;
    let assets = body["assets"].as_object().unwrap();
    assert_eq!(assets.len(), 1);
    let (key, asset) = assets.iter().next().unwrap();
    assert_eq!(asset["type"], "image");
    assert_eq!(asset["format"], "jpeg");
    assert!(!asset["data_base64"].as_str().unwrap().starts_with("data:"));
    // Too wide for TypeSafe at 2240 px: shrunk to its 1568 px limit.
    assert_eq!(dimensions(asset), (1568, 1092));
    assert_eq!(body["state"]["viewport_screenshot"]["image"], json!(key));
    assert_eq!(
        body["state"]["viewport_screenshot"]["size"],
        json!([1568, 1092])
    );
    // The raw screenshot itself never reaches the provider.
    assert!(!body.to_string().contains("\"_screenshot\""));
    assert!(!body.to_string().contains("data:image"));
    assert_eq!(body.to_string().matches(key.as_str()).count(), 2);
}

#[tokio::test]
async fn images_are_off_by_default_and_the_text_request_is_unchanged() {
    let server = MockServer::start(vec![Reply::ok(DONE)]).await;
    let off = client(&server, false);
    assert!(!off.uses_images());
    let default = JevTypeSafeDecisionClient::new("sk", "jev-latest");
    assert!(!default.uses_images());
    let screenshot = data_url(ImageFormat::Jpeg, 1120, 780);
    off.choose(&page(Some(screenshot)), "Finish", &[])
        .await
        .unwrap();
    off.choose(&page(None), "Finish", &[]).await.unwrap();

    let requests = server.requests();
    assert!(requests[0].1.get("assets").is_none());
    assert_eq!(
        without_date(requests[0].1.clone()),
        without_date(requests[1].1.clone())
    );
    for head in server.heads() {
        assert_eq!(images_header(&head), None);
    }
}

#[tokio::test]
async fn a_screenshot_that_cannot_be_fitted_is_left_out() {
    let server = MockServer::start(vec![Reply::ok(DONE)]).await;
    let on = client(&server, true);
    // 20:1 cannot put both sides inside 280..=1568 px.
    let screenshot = data_url(ImageFormat::Png, 2000, 100);
    on.choose(&page(Some(screenshot)), "Finish", &[])
        .await
        .unwrap();
    on.choose(&page(None), "Finish", &[]).await.unwrap();
    for (_, body) in server.requests() {
        assert!(body.get("assets").is_none());
        assert!(body["state"].get("viewport_screenshot").is_none());
    }
    for head in server.heads() {
        assert_eq!(images_header(&head), None);
    }
}

#[tokio::test]
async fn a_rejected_image_request_stops_without_an_action_and_names_the_reason() {
    let server = MockServer::start(vec![Reply::Status(
        400,
        Vec::new(),
        r#"{"detail":"images are not enabled"}"#,
    )])
    .await;
    let error = client(&server, true)
        .choose(
            &page(Some(data_url(ImageFormat::Jpeg, 640, 480))),
            "Finish",
            &[],
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("HTTP 400"), "{error}");
    assert!(
        error.to_string().contains("images are not enabled"),
        "{error}"
    );
    assert_eq!(server.hits(), 1);
}

#[tokio::test]
async fn target_images_preserve_dom_choices_and_gate_without_imaging_the_operation() {
    use crate::jev_prompt::Profile;
    let reply = r#"{"answers":{"operation":{"choice":"DONE","confidence":1.0,"probabilities":{"CLICK":0.0,"TYPE_TEXT":0.0,"DONE":1.0,"BLOCKED":0.0}}}}"#;
    let server =
        MockServer::start(vec![Reply::ok(reply), Reply::ok(reply), Reply::ok(reply)]).await;
    let mut observation = page(Some(data_url(ImageFormat::Png, 1120, 780)));
    observation["actions"] = json!([
        {"kind":"click","node":1,"id":"click:1","label":"Submit","role":"button","rect":{"x":0,"y":0,"w":100,"h":30}},
        {"kind":"fill","node":2,"id":"fill:2","label":"Name","role":"textbox","value":"Ada","value_clipped":true,"rect":{"x":0,"y":40,"w":100,"h":30}}
    ]);
    observation["dom_snapshot"] = json!("- textbox Name [value=\"Ada\"]\n- button Submit");
    for profile in [Profile::Dom, Profile::DomTargets] {
        client(&server, true)
            .with_profile(profile)
            .choose_gated(&observation, "Submit Ada", &[])
            .await
            .unwrap();
    }
    observation["actions"][1]
        .as_object_mut()
        .unwrap()
        .remove("value_clipped");
    client(&server, true)
        .with_profile(Profile::DomTargets)
        .choose_gated(&observation, "Submit Ada", &[])
        .await
        .unwrap();
    let requests = server.requests();
    let shared = &requests[0].1;
    let scoped = &requests[1].1;
    assert!(scoped["state"].get("viewport_screenshot").is_none());
    assert!(scoped["state"].get("candidate_rectangles").is_none());
    assert_eq!(scoped["assets"], shared["assets"]);
    let key = scoped["assets"].as_object().unwrap().keys().next().unwrap();
    let mut gates = 0;
    for (name, original) in shared["questions"].as_object().unwrap() {
        let mut current = scoped["questions"][name].clone();
        if name.ends_with("_target") {
            let instructions = current["instructions"].as_object_mut().unwrap();
            let viewport = instructions.remove("viewport_screenshot").unwrap();
            assert_eq!(viewport["image"], json!(key));
            assert_eq!(viewport, shared["state"]["viewport_screenshot"]);
            assert_eq!(
                instructions.remove("candidate_rectangles").unwrap(),
                shared["state"]["candidate_rectangles"]
            );
        }
        if original["type"] == "noul" {
            gates += 1;
        }
        assert_eq!(&current, original, "question meaning changed: {name}");
    }
    assert!(gates > 0);
    assert_eq!(images_header(&server.heads()[1]).as_deref(), Some("true"));
    // A populated field alone must not remove shared visual evidence.
    assert_eq!(&requests[2].1, shared);
}

#[tokio::test]
async fn target_images_keep_shared_visual_evidence_outside_populated_editable_fields() {
    let server = MockServer::start(vec![Reply::ok(DONE)]).await;
    client(&server, true)
        .with_profile(crate::jev_prompt::Profile::DomTargets)
        .choose(
            &page(Some(data_url(ImageFormat::Png, 1120, 780))),
            "Finish",
            &[],
        )
        .await
        .unwrap();
    let requests = server.requests();
    assert_eq!(requests[0].1["assets"].as_object().unwrap().len(), 1);
    assert!(requests[0].1["state"]["viewport_screenshot"]["image"].is_string());
    assert_eq!(images_header(&server.heads()[0]).as_deref(), Some("true"));
}
