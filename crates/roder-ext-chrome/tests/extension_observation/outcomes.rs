// What an action says about the page it left: the outcome sentence, the new and
// changed controls, and the tab a page is compared on.

#[tokio::test]
async fn an_unchanged_page_leads_with_no_visible_change() {
    let extension = Arc::new(Scripted::default());
    let page = cart(vec![control("c1", "button", "button", "Pay")]);
    extension.reply("page/snapshot", Ok(snapshot_reply(&page)));
    extension.reply(
        "page/click",
        Ok(observed_reply(json!({"ref": "c1"}), &page, 7)),
    );
    let registry = registry(&extension);
    call(&registry, "chrome_page_snapshot", json!({})).await;

    let click = call(&registry, "chrome_click", json!({"ref": "c1"})).await;

    assert!(!click.is_error, "{}", click.text);
    assert_eq!(outcome(&click.text), "No visible change.");
    assert!(
        click.text.lines().next().unwrap().contains("UNTRUSTED"),
        "the untrusted label leads: {}",
        click.text
    );
    assert!(
        at(&click.text, "Outcome: ") < at(&click.text, "Controls")
            && at(&click.text, "Controls") < at(&click.text, "Text:"),
        "outcome, then controls, then text:\n{}",
        click.text
    );
    assert!(
        !click.text.contains("\"controls\""),
        "the model reads lines, not the snapshot's JSON:\n{}",
        click.text
    );
    assert_eq!(
        extension.kinds(),
        ["page/snapshot", "page/click"],
        "an observed action needs no second snapshot"
    );
    assert_eq!(click.data["untrusted"], json!(true));
    assert_eq!(click.data["content"]["action"]["ref"], "c1");
    assert_eq!(click.data["outcome"]["sentence"], "No visible change.");
    assert_eq!(click.data["observation"]["build"], "action-observation");
    assert_eq!(click.data["observation"]["source"], "action_result");
}

#[tokio::test]
async fn a_new_address_is_named_old_to_new() {
    let extension = Arc::new(Scripted::default());
    let before = cart(vec![control("c1", "button", "button", "Pay")]);
    let after = snapshot(
        "https://app.test/receipt",
        "Receipt",
        "Thanks",
        vec![control("c9", "a", "link", "Home")],
    );
    extension.reply("page/snapshot", Ok(snapshot_reply(&before)));
    extension.reply(
        "page/click",
        Ok(observed_reply(json!({"ref": "c1"}), &after, 7)),
    );
    let registry = registry(&extension);
    call(&registry, "chrome_page_snapshot", json!({})).await;

    let click = call(&registry, "chrome_click", json!({"ref": "c1"})).await;

    assert_eq!(
        outcome(&click.text),
        "URL https://app.test/cart -> https://app.test/receipt. Title now \"Receipt\"."
    );
    assert!(
        !click.text.contains("[new]"),
        "a new page marks nothing as new:\n{}",
        click.text
    );
}

#[tokio::test]
async fn changed_and_new_controls_are_counted_and_marked() {
    let extension = Arc::new(Scripted::default());
    let mut agree = control("c2", "input", "checkbox", "");
    agree["ariaName"] = json!("Agree");
    agree["checked"] = json!(false);
    let before = cart(vec![
        control("c1", "button", "button", "Add"),
        agree.clone(),
    ]);
    agree["checked"] = json!(true);
    let after = cart(vec![
        control("c1", "button", "button", "Add"),
        agree,
        control("c3", "a", "link", "Next"),
    ]);
    extension.reply("page/snapshot", Ok(snapshot_reply(&before)));
    extension.reply(
        "page/click",
        Ok(observed_reply(json!({"ref": "c2"}), &after, 7)),
    );
    let registry = registry(&extension);
    call(&registry, "chrome_page_snapshot", json!({})).await;

    let click = call(&registry, "chrome_click", json!({"ref": "c2"})).await;

    assert_eq!(outcome(&click.text), "2 controls changed.");
    let new = line_of(&click.text, "c3 ");
    assert!(
        new.contains("link \"Next\"") && new.contains("[new]"),
        "{new}"
    );
    let flipped = line_of(&click.text, "c2 ");
    assert!(
        flipped.contains("checkbox \"Agree\"")
            && flipped.contains("[checked]")
            && flipped.contains("[changed]"),
        "{flipped}"
    );
    let same = line_of(&click.text, "c1 ");
    assert!(
        !same.contains("[new]") && !same.contains("[changed]"),
        "{same}"
    );
}

#[tokio::test]
async fn text_that_changed_is_named_when_no_control_did() {
    let extension = Arc::new(Scripted::default());
    let before = cart(vec![control("c1", "button", "button", "Add")]);
    let mut after = before.clone();
    after["text"] = json!("Your cart: 1 item");
    extension.reply("page/snapshot", Ok(snapshot_reply(&before)));
    extension.reply(
        "page/click",
        Ok(observed_reply(json!({"ref": "c1"}), &after, 7)),
    );
    let registry = registry(&extension);
    call(&registry, "chrome_page_snapshot", json!({})).await;

    let click = call(&registry, "chrome_click", json!({"ref": "c1"})).await;

    assert_eq!(outcome(&click.text), "Page text changed.");
}

#[tokio::test]
async fn the_first_action_with_nothing_to_compare_says_so() {
    let extension = Arc::new(Scripted::default());
    let page = cart(vec![control("c1", "button", "button", "Pay")]);
    extension.reply(
        "page/click",
        Ok(observed_reply(json!({"ref": "c1"}), &page, 7)),
    );
    let registry = registry(&extension);

    let click = call(&registry, "chrome_click", json!({"ref": "c1"})).await;

    assert_eq!(
        outcome(&click.text),
        "No earlier observation of this tab to compare with."
    );
    assert_eq!(click.data["outcome"]["compared"], json!(false));
}

#[tokio::test]
async fn another_tab_is_not_compared_with_the_last_one() {
    let extension = Arc::new(Scripted::default());
    let page = cart(vec![control("c1", "button", "button", "Pay")]);
    let other = snapshot(
        "https://other.test/",
        "Other",
        "Elsewhere",
        vec![control("c1", "button", "button", "Pay")],
    );
    extension.reply(
        "page/click",
        Ok(observed_reply(json!({"ref": "c1"}), &page, 7)),
    );
    extension.reply(
        "page/click",
        Ok(observed_reply(json!({"ref": "c1"}), &other, 8)),
    );
    let registry = registry(&extension);
    call(&registry, "chrome_click", json!({"ref": "c1"})).await;

    // The active tab is another one now: claiming the old page became this
    // one would be false.
    let click = call(&registry, "chrome_click", json!({"ref": "c1"})).await;

    assert_eq!(
        outcome(&click.text),
        "No earlier observation of this tab to compare with."
    );
}

#[tokio::test]
async fn a_navigation_is_read_as_controls_then_text_and_becomes_the_baseline() {
    let extension = Arc::new(Scripted::default());
    let page = cart(vec![control("c1", "button", "button", "Pay")]);
    extension.reply(
        "tab/navigate",
        Ok(observed_reply(
            json!({"id": 7, "url": "https://app.test/cart"}),
            &page,
            7,
        )),
    );
    extension.reply(
        "page/click",
        Ok(observed_reply(json!({"ref": "c1"}), &page, 7)),
    );
    let registry = registry(&extension);

    let navigated = call(
        &registry,
        "chrome_navigate",
        json!({"url": "https://app.test/cart"}),
    )
    .await;

    assert!(!navigated.is_error, "{}", navigated.text);
    assert!(
        !navigated.text.contains("Outcome: "),
        "a navigation has no earlier page to be an outcome of:\n{}",
        navigated.text
    );
    assert!(
        at(&navigated.text, "Controls") < at(&navigated.text, "Text:"),
        "{}",
        navigated.text
    );
    assert!(
        navigated.text.contains("c1 button \"Pay\""),
        "{}",
        navigated.text
    );
    let click = call(&registry, "chrome_click", json!({"ref": "c1"})).await;
    assert_eq!(outcome(&click.text), "No visible change.");
}

#[tokio::test]
async fn a_page_seen_on_a_tab_is_compared_whichever_way_the_tab_was_named() {
    let extension = Arc::new(Scripted::default());
    let before = cart(vec![control("c1", "button", "button", "Add")]);
    let after = cart(vec![
        control("c1", "button", "button", "Add"),
        control("c2", "a", "link", "Checkout"),
    ]);
    extension.reply("page/snapshot", Ok(snapshot_reply(&before)));
    // The extension says which tab it acted on, whether or not the call did.
    extension.reply(
        "page/click",
        Ok(observed_reply(json!({"ref": "c1"}), &after, 7)),
    );
    let registry = registry(&extension);
    call(&registry, "chrome_page_snapshot", json!({"tabId": 7})).await;

    let click = call(&registry, "chrome_click", json!({"ref": "c1"})).await;

    assert_eq!(
        outcome(&click.text),
        "1 control changed.",
        "tab 7 was read, then tab 7 was clicked:\n{}",
        click.text
    );
}

#[tokio::test]
async fn a_click_on_a_named_tab_is_not_compared_with_a_page_of_no_known_tab() {
    let extension = Arc::new(Scripted::default());
    let active = cart(vec![control("c1", "button", "button", "Pay")]);
    let named = snapshot(
        "https://other.test/",
        "Other",
        "Elsewhere",
        vec![control("c1", "button", "button", "Pay")],
    );
    extension.reply("page/snapshot", Ok(snapshot_reply(&active)));
    extension.reply(
        "page/click",
        Ok(observed_reply(json!({"ref": "c1"}), &named, 7)),
    );
    let registry = registry(&extension);
    call(&registry, "chrome_page_snapshot", json!({})).await;

    let click = call(&registry, "chrome_click", json!({"ref": "c1", "tabId": 7})).await;

    assert_eq!(
        outcome(&click.text),
        "No earlier observation of this tab to compare with.",
        "the snapshot may have been of another tab; no navigation is claimed:\n{}",
        click.text
    );
}
