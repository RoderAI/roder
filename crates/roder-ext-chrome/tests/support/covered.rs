// A target another element covers is refused, and the refusal says what to do
// next in terms of the tool that was called. The `chrome_*` tools take a
// selector, text or ref and no coordinates, so what they are told never
// involves x/y; the direct tools (`jev_tab_*`) take x/y and keep that advice,
// except `type`, which has no coordinates either. The cover's name is page
// text: one line, capped, without quote marks of its own, secrets taken out.
struct Redact;
impl roder_ext_chrome::direct::DirectGuard for Redact {
    fn scrub(&self, text: &str) -> String {
        text.replace("hunter2", "[REDACTED]")
    }
}

/// The part of `text` between the first pair of double quotes.
fn quoted(text: &str) -> &str {
    let (_, rest) = text.split_once('"').expect("a quoted cover name");
    rest.split_once('"').expect("a closing quote").0
}

/// The cover's name in a "covered by "NAME"; ..." refusal: what the refusal
/// closes with its own quote.
fn named_cover(text: &str) -> &str {
    let (_, rest) = text.split_once("covered by \"").expect("a named cover");
    rest.split_once("\";")
        .expect("the name is closed by the refusal's own quote")
        .0
}

async fn covered_page_tab(browser: &Browser, page: &str) -> DirectTab {
    let targets: Value = reqwest::get(format!("{}/json", browser.endpoint))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let target = targets
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["type"] == "page" && t["url"] == page)
        .unwrap_or_else(|| panic!("no tab is on {page}: {targets}"));
    DirectTab::Page {
        websocket: target["webSocketDebuggerUrl"].as_str().unwrap().into(),
        target_id: target["id"].as_str().unwrap().into(),
    }
}

async fn covered_targets_get_advice_their_tool_can_follow(
    registry: &ToolRegistry,
    browser: &Browser,
    url: &str,
) {
    let page = format!("{url}covered");
    let opened = call(registry, "chrome_navigate", json!({"url": page})).await;
    assert!(!opened.is_error, "{}", opened.text);

    // The premise: no chrome tool that can be refused this way takes x/y.
    for tool in ["chrome_click", "chrome_type", "chrome_scroll"] {
        let spec = registry.get(tool).unwrap().spec();
        for key in ["x", "y"] {
            assert!(
                spec.parameters["properties"].get(key).is_none(),
                "{tool} takes {key}"
            );
        }
    }

    // chrome_click and chrome_type: named cover, advice with no coordinates.
    let calls = [
        ("chrome_click", json!({"selector": "#pay"})),
        (
            "chrome_type",
            json!({"selector": "#card-name", "text": "Ada"}),
        ),
    ];
    for (tool, args) in calls {
        let refused = call(registry, tool, args).await;
        assert!(refused.is_error, "{tool}: {}", refused.text);
        let text = &refused.text;
        assert!(text.contains("is covered"), "{tool}: {text}");
        assert!(text.contains("nothing was pressed"), "{tool}: {text}");
        assert!(!text.contains("x/y"), "{tool} has no coordinates: {text}");
        assert!(
            text.contains("scroll") && text.contains("different target"),
            "{tool} must offer actions it can take: {text}"
        );
        let name = quoted(text);
        assert!(name.starts_with("Cookie notice"), "{tool}: {text}");
        assert!(
            name.chars().count() <= 61,
            "{tool}: cover name is capped: {text}"
        );
        assert!(!name.contains('\n'), "{tool}: {text}");
        assert!(
            !text.contains("\"Accept all\""),
            "{tool}: the page's own quote marks must not survive: {text}"
        );
    }
    assert_eq!(
        eval(registry, "document.querySelector('#card-name').value").await,
        "",
        "a covered field is not typed into"
    );

    // The direct tools: x/y stays in the advice, except for `type`.
    let tab = covered_page_tab(browser, &page).await;
    let mut direct = DirectSession::attach(&tab, Arc::new(Redact), false)
        .await
        .unwrap();
    let looked = direct.run("look", &json!({})).await;
    let elements = looked.data["page"]["elements"].as_array().unwrap();
    let pay = elements.iter().find(|e| e["label"] == "Pay now").unwrap()["ref"].clone();
    let field = elements
        .iter()
        .find(|e| e["label"] == "Name on card")
        .unwrap()["ref"]
        .clone();
    let clicked = direct.run("click", &json!({"ref": pay})).await;
    assert!(clicked.is_error, "{}", clicked.text);
    assert!(clicked.text.contains("press at x/y"), "{}", clicked.text);
    assert!(clicked.text.contains("is covered at ("), "{}", clicked.text);
    assert!(clicked.text.contains("[REDACTED]"), "{}", clicked.text);
    assert!(!clicked.text.contains("hunter2"), "{}", clicked.text);
    let typed = direct
        .run("type", &json!({"ref": field, "text": "Ada"}))
        .await;
    assert!(typed.is_error, "{}", typed.text);
    assert!(typed.text.contains("is covered"), "{}", typed.text);
    assert!(
        !typed.text.contains("x/y"),
        "type has no x/y: {}",
        typed.text
    );
    assert!(!typed.text.contains("hunter2"), "{}", typed.text);

    // A key presses what has focus, so it is refused for a covered control
    // too, and the cover is named as the other refusals name it: page text,
    // scrubbed, on one line, without quote marks of its own, capped.
    eval(registry, "document.querySelector('#pay').focus()").await;
    for key in ["Enter", "Space"] {
        let pressed = direct.run("key", &json!({"key": key})).await;
        let text = &pressed.text;
        assert!(pressed.is_error, "{key}: {text}");
        assert!(text.contains("is covered by"), "{key}: {text}");
        assert!(text.contains("nothing was pressed"), "{key}: {text}");
        assert!(
            !text.contains("hunter2"),
            "{key}: the secret is scrubbed: {text}"
        );
        assert!(!text.contains('\n'), "{key}: one line: {text}");
        let name = named_cover(text);
        assert!(
            name.starts_with("Cookie notice [REDACTED] 'Accept all'"),
            "{key}: {text}"
        );
        assert!(
            !name.contains('"'),
            "{key}: no quote marks of its own: {text}"
        );
        assert!(name.chars().count() <= 61, "{key}: capped: {text}");
    }

    // What has focus is named from the page too: the label is scrubbed in
    // the text and in the data.
    eval(registry, "document.querySelector('#tip').focus()").await;
    let pressed = direct.run("key", &json!({"key": "Space"})).await;
    assert!(!pressed.is_error, "{}", pressed.text);
    assert!(pressed.text.contains("Tip [REDACTED]"), "{}", pressed.text);
    assert!(
        !pressed.text.contains("hunter2") && !pressed.data.to_string().contains("hunter2"),
        "{} {}",
        pressed.text,
        pressed.data
    );
    assert_eq!(
        eval(
            registry,
            "document.querySelector('#pay').dataset.pressed ?? null"
        )
        .await,
        Value::Null
    );
}
