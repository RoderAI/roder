//! Real CDP computer execution after a Decisions screenshot choice.
use super::Harness;
use crate::{ComputerCandidate, JevDecisionTransport, OpenAiDecisionsClient};
use async_trait::async_trait;
use roder_api::computer::{ComputerAction, ComputerActions};
use roder_ext_chrome::direct::{DirectSession, DirectTab, OpenGuard};
use serde_json::{Value, json};
use std::sync::Arc;

struct Scripted;
#[async_trait]
impl JevDecisionTransport for Scripted {
    async fn decide(&self, request: &Value) -> anyhow::Result<Value> {
        assert_eq!(request["model"], "gpt-6-luna");
        assert!(
            request["input"][0]["content"][1]["image_url"]
                .as_str()
                .unwrap()
                .starts_with("data:image/jpeg;base64,")
        );
        assert_eq!(
            request["questions"][0]["choices"].as_array().unwrap().len(),
            3
        );
        Ok(
            json!({"answers":[{"type":"choice","name":"computer_action", "choice":"right", "confidence":1.0,
            "probabilities":[{"value":"blocked","probability":0.0},{"value":"left","probability":0.0},{"value":"right","probability":1.0}]}], "usage":{"input_tokens":123}}),
        )
    }
}

async fn canvas(client: OpenAiDecisionsClient, harness: Harness) {
    let mut page = harness.open("canvas-pad.html").await.unwrap();
    let rect = page.evaluate("(() => {const r=document.querySelector('canvas').getBoundingClientRect(); return {x:r.left,y:r.top}})()").await.unwrap();
    let mut session = DirectSession::attach(
        &DirectTab::Target {
            endpoint: harness.endpoint().into(),
            target_id: page.target_id().into(),
        },
        Arc::new(OpenGuard),
        false,
    )
    .await
    .unwrap();
    let screenshot = session.run("screenshot", &json!({})).await;
    assert!(!screenshot.is_error, "{}", screenshot.text);
    let candidates = [("left", 60.0), ("right", 335.0)]
        .into_iter()
        .map(|(id, dx)| {
            let x = rect["x"].as_f64().unwrap() + dx;
            let y = rect["y"].as_f64().unwrap() + 45.0;
            ComputerCandidate {
                id: id.into(),
                description: format!("Click at ({x}, {y})"),
                actions: ComputerActions {
                    actions: vec![ComputerAction::Click {
                        x,
                        y,
                        button: "left".into(),
                        keys: Vec::new(),
                    }],
                },
            }
        })
        .collect::<Vec<_>>();
    let chosen = client
        .choose_computer(
            "Press the OK button drawn on the canvas.",
            screenshot.image.as_deref().unwrap(),
            &candidates,
        )
        .await
        .unwrap();
    let result = session
        .run_computer(
            chosen
                .actions
                .as_ref()
                .expect("model must choose a useful action"),
        )
        .await;
    assert!(!result.is_error, "{}", result.text);
    assert_eq!(result.data["completed_actions"], 1);
    assert_eq!(
        page.evaluate("window.canvasOk === true").await.unwrap(),
        json!(true)
    );
    eprintln!(
        "Decisions computer outcome: canvas OK pressed; choice={}, confidence={}, usage={}",
        chosen.choice, chosen.confidence, chosen.usage
    );
    page.close().await.unwrap();
}

#[tokio::test]
async fn decisions_computer_choice_executes_through_real_cdp() {
    canvas(
        OpenAiDecisionsClient::with_transport(Arc::new(Scripted)),
        harness_or_skip!(),
    )
    .await;
}

#[tokio::test]
#[ignore = "live OpenAI Decisions screenshot request; requires an API key and Chrome"]
async fn decisions_live_computer_canvas() {
    let key = crate::runner::env_value("OPENAI_API_KEY")
        .or_else(|| roder_config::provider_api_key("openai"))
        .expect("live Decisions validation requires an OpenAI API key");
    canvas(
        OpenAiDecisionsClient::new(key),
        Harness::start()
            .await
            .expect("live computer validation requires Chrome"),
    )
    .await;
}

struct BrowserEvidence {
    expect_image: bool,
}
#[async_trait]
impl JevDecisionTransport for BrowserEvidence {
    async fn decide(&self, request: &Value) -> anyhow::Result<Value> {
        assert_eq!(request["input"].is_array(), self.expect_image);
        if self.expect_image {
            assert!(
                request["input"][0]["content"][1]["image_url"]
                    .as_str()
                    .unwrap()
                    .starts_with("data:image/jpeg;base64,")
            );
        }
        let answers=request["questions"].as_array().unwrap().iter().map(|q| {
            let choices=q["choices"].as_array().unwrap();
            let selected=choices.iter().find(|c|c["value"]=="DONE").unwrap_or(&choices[0])["value"].clone();
            json!({"type":"choice","name":q["name"],"choice":selected,"confidence":1.0,"probabilities":choices.iter().map(|c|json!({"value":c["value"],"probability":if c["value"]==selected {1.0} else {0.0}})).collect::<Vec<_>>()})
        }).collect::<Vec<_>>();
        Ok(json!({"answers":answers,"usage":{"input_tokens":10,"output_tokens":0}}))
    }
}

#[tokio::test]
async fn decisions_browser_captures_visual_evidence_but_suppresses_secret_pages() {
    let harness = harness_or_skip!();
    for (fixture, expect_image) in [("products.html", true), ("secrets.html", false)] {
        let page = harness.open(fixture).await.unwrap();
        let client =
            OpenAiDecisionsClient::with_transport(Arc::new(BrowserEvidence { expect_image }));
        let config = crate::JevEngineConfig::new("Inspect this page", Arc::new(client))
            .with_cookie_banner_refusal(false);
        let mut engine = crate::JevEngine::start(Box::new(page), config)
            .await
            .unwrap();
        let result = engine.run(std::time::Duration::from_secs(10)).await;
        engine.close().await.unwrap();
        assert_eq!(result.status, crate::JevStatus::Done);
        assert!(
            !serde_json::to_string(&result)
                .unwrap()
                .contains("data:image")
        );
    }
}
