// A screenshot that cannot be taken, or a browser that cannot be reached, is
// still a result with a picture.

/// Remembers what was typed into secret fields, as an owner's guard does.
#[derive(Default)]
struct Remembering(Mutex<Vec<String>>);
impl DirectGuard for Remembering {
    fn remember_secret(&self, value: &str) {
        self.0.lock().unwrap().push(value.to_string());
    }
    fn scrub(&self, text: &str) -> String {
        self.0
            .lock()
            .unwrap()
            .iter()
            .fold(text.to_string(), |text, secret| {
                text.replace(secret, "[REDACTED]")
            })
    }
}

#[tokio::test]
async fn a_screenshot_that_cannot_be_taken_is_replaced_by_a_placeholder() {
    let Some(browser) = support::Browser::start().await.unwrap() else {
        return;
    };
    let traps = Traps::start().await;
    let (websocket, target_id) = first_page(&browser).await;
    let tab = DirectTab::Page {
        websocket,
        target_id,
    };
    let mut session = DirectSession::attach(&tab, Arc::new(Remembering::default()), false)
        .await
        .unwrap();
    let loaded = session
        .run("navigate", &json!({"url": format!("{}secret", traps.url)}))
        .await;
    assert!(!loaded.is_error, "{}", loaded.text);
    // The page echoes the typed password in its text, so the guard withholds
    // the picture: nothing may show a secret.
    let step = session
        .run_computer(&actions(json!([
            {"type":"click","button":"left","x":60,"y":40},
            {"type":"type","text":"hunter2-secret"},
        ])))
        .await;
    let result = tool_result("id", "computer", &step);
    let image = result.data[VIEW_IMAGE_DISPLAY_KEY]["image_url"]
        .as_str()
        .unwrap_or_else(|| panic!("a native result always carries an image: {}", result.text));
    assert!(image.starts_with("data:image/png;base64,"), "{image}");
    assert_eq!(result.data["screenshot_unavailable"], true);
    assert!(
        !result.is_error,
        "the actions worked; only the picture is missing: {}",
        result.text
    );
    let notes = notes(&result.data).join("\n");
    assert!(notes.contains("Screenshot unavailable"), "{notes}");
    assert!(
        !result.text.contains("hunter2-secret")
            && !result.data.to_string().contains("hunter2-secret"),
        "{}",
        result.text
    );
}

struct NoTab;
#[async_trait]
impl DirectBinding for NoTab {
    async fn lease(
        &self,
        _: &ToolExecutionContext,
        _: &ToolCall,
    ) -> Result<Box<dyn DirectLease>, String> {
        Err("no browser is bound to this thread".into())
    }
}

#[tokio::test]
async fn a_call_that_cannot_reach_its_browser_still_returns_a_screenshot() {
    let tool = ComputerTool::new(Arc::new(NoTab));
    let args = json!({"actions": [{"type": "screenshot"}]});
    let result = tool
        .execute(
            ToolExecutionContext::new("native-notes", "turn", PolicyMode::Bypass),
            ToolCall {
                id: "call".into(),
                name: "computer".into(),
                arguments: args.clone(),
                raw_arguments: args.to_string(),
                thread_id: "native-notes".into(),
                turn_id: "turn".into(),
            },
        )
        .await
        .expect("a failed call is a result with a screenshot, not an executor error");
    assert!(result.is_error);
    assert!(
        result.text.contains("no browser is bound"),
        "{}",
        result.text
    );
    assert!(
        result.data[VIEW_IMAGE_DISPLAY_KEY]["image_url"]
            .as_str()
            .is_some_and(|image| image.starts_with("data:image/png;base64,")),
        "{}",
        result.data
    );
}
