// An extension build that answers an action without observing the page: one
// `page/snapshot` after a short delay stands in for the observation.

#[tokio::test]
async fn an_old_build_gets_one_snapshot_after_the_delay() {
    let extension = Arc::new(Scripted::default());
    let before = cart(vec![control("c1", "button", "button", "Add")]);
    let after = cart(vec![
        control("c1", "button", "button", "Add"),
        control("c2", "a", "link", "Checkout"),
    ]);
    extension.reply("page/snapshot", Ok(snapshot_reply(&before)));
    // The build before `action-observation` answers with what it did, only.
    extension.reply("page/click", Ok(json!({"ok": true, "ref": "c1"})));
    extension.reply("page/snapshot", Ok(snapshot_reply(&after)));
    let registry = registry(&extension);
    call(&registry, "chrome_page_snapshot", json!({"tabId": 3})).await;

    let click = call(&registry, "chrome_click", json!({"ref": "c1", "tabId": 3})).await;

    let log = extension.log();
    assert_eq!(
        extension.kinds(),
        ["page/snapshot", "page/click", "page/snapshot"],
        "one snapshot follows an action that came back without an observation"
    );
    let waited = log[2].sent_at.duration_since(log[1].answered_at);
    assert!(
        waited >= Duration::from_millis(150),
        "the page gets time to act on the click first, waited {waited:?}"
    );
    assert!(waited < Duration::from_secs(2), "waited {waited:?}");
    assert_eq!(log[2].params["tabId"], json!(3), "the same tab is read");
    assert!(
        !log[2].params["include"]
            .as_array()
            .unwrap()
            .contains(&json!("boxes")),
        "boxes are never used, so they are not fetched: {}",
        log[2].params
    );
    assert!(!click.is_error, "{}", click.text);
    assert_eq!(outcome(&click.text), "1 control changed.");
    assert!(
        click.text.contains("c2 link \"Checkout\""),
        "{}",
        click.text
    );
    assert_eq!(click.data["observation"]["build"], "legacy");
    assert_eq!(click.data["observation"]["source"], "snapshot_fallback");
    assert_eq!(
        click.data["content"]["ref"], "c1",
        "the action's own reply is kept"
    );
    let lower = click.text.to_lowercase();
    assert!(
        !lower.contains("legacy") && !lower.contains("old build") && !lower.contains("fallback"),
        "which build answered is data, not text:\n{}",
        click.text
    );
}

#[tokio::test]
async fn every_page_action_of_an_old_build_is_observed() {
    for (tool, kind, args) in [
        (
            "chrome_type",
            "page/type",
            json!({"ref": "c1", "text": "hi"}),
        ),
        ("chrome_keypress", "page/keypress", json!({"key": "Enter"})),
        ("chrome_scroll", "page/scroll", json!({"dy": 400})),
        (
            "chrome_select",
            "page/select",
            json!({"selector": "#plan", "value": "growth"}),
        ),
    ] {
        let extension = Arc::new(Scripted::default());
        let page = cart(vec![control("c1", "input", "textbox", "")]);
        extension.reply("page/snapshot", Ok(snapshot_reply(&page)));
        extension.reply(kind, Ok(json!({"ok": true})));
        extension.reply("page/snapshot", Ok(snapshot_reply(&page)));
        let registry = registry(&extension);
        call(&registry, "chrome_page_snapshot", json!({})).await;

        let result = call(&registry, tool, args).await;

        assert_eq!(
            extension.kinds(),
            ["page/snapshot", kind, "page/snapshot"],
            "{tool}"
        );
        assert_eq!(outcome(&result.text), "No visible change.", "{tool}");
        assert_eq!(result.data["observation"]["build"], "legacy", "{tool}");
    }
}

#[tokio::test]
async fn an_old_build_whose_snapshot_fails_says_so_and_asks_once() {
    let extension = Arc::new(Scripted::default());
    let page = cart(vec![control("c1", "button", "button", "Add")]);
    extension.reply("page/snapshot", Ok(snapshot_reply(&page)));
    extension.reply("page/click", Ok(json!({"ok": true, "ref": "c1"})));
    extension.reply(
        "page/snapshot",
        Err(ChromeError::Remote("site permission was revoked".into())),
    );
    let registry = registry(&extension);
    call(&registry, "chrome_page_snapshot", json!({})).await;

    let click = call(&registry, "chrome_click", json!({"ref": "c1"})).await;

    assert_eq!(
        extension.kinds(),
        ["page/snapshot", "page/click", "page/snapshot"],
        "no second try"
    );
    assert!(!click.is_error, "the click itself ran: {}", click.text);
    assert!(
        click.text.contains("site permission was revoked")
            && click.text.contains("chrome_page_snapshot"),
        "the model is told why there is no page and what to do:\n{}",
        click.text
    );
    assert!(
        !click.text.contains("Outcome: No visible change"),
        "an unobserved page is not reported unchanged:\n{}",
        click.text
    );
    assert_eq!(click.data["observation"]["build"], "legacy");
    assert_eq!(click.data["observation"]["source"], "unavailable");
    assert!(
        click.data["observation"]["error"]
            .as_str()
            .unwrap()
            .contains("site permission was revoked")
    );
    assert_eq!(click.data["content"]["ref"], "c1");
}
