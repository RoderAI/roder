// The same scripted steps on one page, through Roder Desktop's browser (real
// input, a real page) and through a scripted extension that answers with what
// the extension's snapshot would list for that page, must say the same thing
// about what each step did: the `Outcome:` sentence. Then the Desktop look says
// when it cut the page's text.
fn outcome_sentence(text: &str) -> String {
    text.lines()
        .find_map(|line| line.strip_prefix("Outcome: "))
        .unwrap_or_else(|| panic!("no Outcome line in:\n{text}"))
        .to_string()
}

/// The page of `fixtures/outcome.html` as the extension's snapshot lists it.
fn extension_view(url: &str, text: &str, agreed: bool) -> Value {
    let mut agree = scripted::control("c3", "input", "checkbox", "");
    agree["ariaName"] = json!("Agree");
    agree["checked"] = json!(agreed);
    let mut next = scripted::control("c4", "a", "link", "Next");
    next["href"] = json!(format!("{url}#next"));
    scripted::snapshot(
        url,
        "Outcome fixture",
        text,
        vec![
            scripted::control("c1", "button", "button", "Dead"),
            scripted::control("c2", "button", "button", "Add item"),
            agree,
            next,
        ],
    )
}

async fn desktop_and_extension_say_the_same(desktop: &ToolRegistry, url: &str) {
    let page = format!("{url}outcome");
    let opened = call(desktop, "chrome_navigate", json!({"url": page})).await;
    assert!(!opened.is_error, "{}", opened.text);
    assert!(
        !opened.text.contains("text cut at"),
        "a short page has no marker: {}",
        opened.text
    );
    // A page is not the effect of an input: a navigation and a look carry no
    // outcome line, as on the extension.
    let no_outcome = |result: &ToolResult| {
        !result.text.lines().any(|line| line.starts_with("Outcome: "))
            && result.data["outcome"].is_null()
    };
    assert!(no_outcome(&opened), "{}", opened.text);
    let looked = call(desktop, "chrome_page_snapshot", json!({})).await;
    assert!(!looked.is_error, "{}", looked.text);
    assert!(no_outcome(&looked), "{}", looked.text);
    // The page script counts the characters of a text only when it cut it.
    assert!(
        looked.data["page"]["text_total"].is_null() && looked.data["page"]["text_cut"] == false,
        "{}",
        looked.data["page"]
    );

    let base = "Outcome fixture Dead Add item Agree Next";
    let with_item = "Outcome fixture Dead Add item Item 1 Agree Next";
    let extension = Arc::new(scripted::Scripted::default());
    let steps = [
        // (the control pressed, the page the extension then lists, the sentence)
        (
            "#dead",
            extension_view(&page, base, false),
            "No visible change.".to_string(),
        ),
        (
            "#add",
            extension_view(&page, with_item, false),
            "Page text changed.".to_string(),
        ),
        (
            "#agree",
            extension_view(&page, with_item, true),
            "1 control changed.".to_string(),
        ),
        (
            "#next",
            extension_view(&format!("{page}#next"), with_item, true),
            format!("URL {page} -> {page}#next."),
        ),
    ];
    let mut paired = ToolRegistry::default();
    ChromeToolContributor::with_controller(extension.clone())
        .contribute(&mut paired)
        .unwrap();
    extension.reply(
        "page/snapshot",
        Ok(scripted::snapshot_reply(&extension_view(
            &page, base, false,
        ))),
    );
    let seeded = call(&paired, "chrome_page_snapshot", json!({})).await;
    assert!(!seeded.is_error, "{}", seeded.text);

    for (selector, extension_after, expected) in steps {
        let on_desktop = call(desktop, "chrome_click", json!({"selector": selector})).await;
        assert!(!on_desktop.is_error, "{selector}: {}", on_desktop.text);
        extension.reply(
            "page/click",
            Ok(scripted::observed_reply(
                json!({"ref": "c1"}),
                &extension_after,
                1,
            )),
        );
        let on_extension = call(&paired, "chrome_click", json!({"selector": selector})).await;
        assert!(!on_extension.is_error, "{selector}: {}", on_extension.text);
        assert_eq!(
            outcome_sentence(&on_desktop.text),
            expected,
            "Desktop, {selector}:\n{}",
            on_desktop.text
        );
        assert_eq!(
            outcome_sentence(&on_extension.text),
            expected,
            "extension, {selector}:\n{}",
            on_extension.text
        );
        assert_eq!(on_desktop.data["outcome"]["sentence"], json!(expected));
        // The label that says page words are untrusted comes first, then the
        // outcome, as on the extension path.
        let mut lines = on_desktop.text.lines();
        assert!(
            lines.next().is_some_and(|l| l.contains("UNTRUSTED"))
                && lines.next().is_some_and(|l| l.starts_with("Outcome: ")),
            "the untrusted label, then the outcome, lead the Desktop result:\n{}",
            on_desktop.text
        );
        assert_eq!(
            on_extension
                .text
                .lines()
                .nth(1)
                .map(|l| l.starts_with("Outcome: ")),
            Some(true),
            "the extension's result has the same two lines first:\n{}",
            on_extension.text
        );
    }

    // A page with more text than a look carries says so, and how much it has.
    eval(
        desktop,
        "document.body.append(Object.assign(document.createElement('p'), {textContent: 'x'.repeat(5000)})); true",
    )
    .await;
    let long = call(desktop, "chrome_page_snapshot", json!({})).await;
    assert!(!long.is_error, "{}", long.text);
    let marker = long
        .text
        .lines()
        .find(|line| line.starts_with("… text cut at "))
        .unwrap_or_else(|| panic!("no text-cut marker in:\n{}", long.text));
    assert!(
        marker.starts_with("… text cut at 3000 chars (the page text is ")
            && long.data["page"]["text_total"].as_u64().unwrap() > 5000,
        "{marker}"
    );

    // Its total counts characters, not UTF-16 units: a code point outside the
    // basic plane is one.
    eval(
        desktop,
        "document.body.append(Object.assign(document.createElement('p'), \
         {textContent: String.fromCodePoint(0x1F600).repeat(2500)})); true",
    )
    .await;
    let counted = eval(
        desktop,
        "(() => { const t = document.body.innerText.replace(/[ \\t]+/g, ' ') \
         .replace(/\\n\\s*\\n+/g, '\\n').trim(); return [[...t].length, t.length]; })()",
    )
    .await;
    let (points, units) = (counted[0].as_u64().unwrap(), counted[1].as_u64().unwrap());
    assert!(units >= points + 2500, "{counted}");
    let astral = call(desktop, "chrome_page_snapshot", json!({})).await;
    assert_eq!(
        astral.data["page"]["text_total"].as_u64(),
        Some(points),
        "{}",
        astral.text
    );

    // A change past the start of a long page is not reported as nothing: the
    // look keeps only the start of the text, and the sentence says so.
    eval(
        desktop,
        "const b = document.createElement('button'); b.id = 'report'; b.textContent = 'Report'; \
         b.onclick = () => document.body.append(Object.assign(document.createElement('p'), \
         {textContent: 'Late error: payment declined'})); document.body.prepend(b); \
         window.scrollTo(0, 0); true",
    )
    .await;
    let late = call(desktop, "chrome_click", json!({"selector": "#report"})).await;
    assert!(!late.is_error, "{}", late.text);
    assert_eq!(
        outcome_sentence(&late.text),
        "No visible change in the controls or in the start of the page text (the text is cut, \
         so later changes are not compared).",
        "{}",
        late.text
    );
    assert_eq!(late.data["outcome"]["changed"], json!(false));
}
