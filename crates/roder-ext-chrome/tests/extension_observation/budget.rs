// The output budget: controls before text, text cut first, controls last, and
// a visible count of whatever is left out.

#[tokio::test]
async fn controls_come_before_the_text_and_all_of_them_survive_a_long_page() {
    let extension = Arc::new(Scripted::default());
    // A long page that fits the line budget: 120 controls, and the 12,000
    // characters of text the extension keeps of a page.
    let controls: Vec<Value> = (1..=120)
        .map(|n| {
            control(
                &format!("c{n}"),
                "button",
                "button",
                &format!("Action {n} {}", "y".repeat(60)),
            )
        })
        .collect();
    let page = snapshot(
        "https://app.test/long",
        "Long",
        &format!("{}abcde", "abcd ".repeat(2399)),
        controls,
    );
    extension.reply("page/snapshot", Ok(snapshot_reply(&page)));
    let registry = registry(&extension);

    let looked = call(&registry, "chrome_page_snapshot", json!({})).await;

    let text = &looked.text;
    assert!(
        text.chars().count() <= CAP && text.lines().count() <= LINES,
        "{} chars, {} lines",
        text.chars().count(),
        text.lines().count()
    );
    assert!(
        text.contains("c120 button \"Action 120 "),
        "the last control is listed:\n{}",
        &text[..400.min(text.len())]
    );
    assert!(at(text, "Controls") < at(text, "Text:"));
    let marker = line_of(text, "… text cut at");
    assert!(
        marker.contains("(the page text is at least 12000 chars)"),
        "{marker}"
    );
    assert!(
        !text.contains("more controls not listed"),
        "none were dropped"
    );
    assert!(
        text.trim_end().ends_with(marker),
        "the marker closes the text:\n{}",
        &text[text.len().saturating_sub(300)..]
    );
}

#[tokio::test]
async fn controls_are_cut_last_and_the_omitted_count_is_shown() {
    let extension = Arc::new(Scripted::default());
    let controls: Vec<Value> = (1..=400)
        .map(|n| {
            control(
                &format!("c{n}"),
                "button",
                "button",
                &format!("Action {n} {}", "y".repeat(60)),
            )
        })
        .collect();
    let page = snapshot(
        "https://app.test/huge",
        "Huge",
        &format!("{}abcde", "abcd ".repeat(2399)),
        controls,
    );
    extension.reply("page/snapshot", Ok(snapshot_reply(&page)));
    let registry = registry(&extension);

    let looked = call(&registry, "chrome_page_snapshot", json!({})).await;

    let text = &looked.text;
    assert!(
        text.chars().count() <= CAP && text.lines().count() <= LINES,
        "{} chars, {} lines",
        text.chars().count(),
        text.lines().count()
    );
    let listed = text
        .lines()
        .filter(|line| line.starts_with('c') && line.contains(" button \""))
        .count();
    assert!(listed > 100 && listed < 400, "{listed} controls listed");
    let marker = line_of(text, "… ");
    let omitted: usize = line_of(text, "… ")
        .trim_start_matches("… ")
        .split(' ')
        .next()
        .unwrap()
        .parse()
        .unwrap_or_else(|_| panic!("a count in {marker:?}"));
    assert!(marker.contains("more controls not listed"), "{marker}");
    assert_eq!(listed + omitted, 400, "the omitted count is exact");
    assert!(
        text.contains("… text cut at 0 chars"),
        "the text went first, all of it:\n{}",
        &text[text.len().saturating_sub(300)..]
    );
}

/// The controls and omission lines of a rendering: how many control lines are
/// listed, and the number its omission line gives.
fn controls_listed_and_omitted(text: &str, label: &str) -> (usize, usize) {
    let listed = text
        .lines()
        .filter(|line| line.starts_with('c') && line.contains(&format!(" button \"{label}")))
        .count();
    let omitted = text
        .lines()
        .find(|line| line.ends_with("more controls not listed"))
        .map(|line| {
            line.trim_start_matches("… ")
                .split(' ')
                .next()
                .unwrap()
                .parse()
                .unwrap()
        })
        .unwrap_or(0);
    (listed, omitted)
}

#[tokio::test]
async fn a_page_of_many_short_controls_is_cut_by_lines_and_keeps_the_outcome_first() {
    let extension = Arc::new(Scripted::default());
    // 190 short controls come to under 5,000 characters: the character cap is
    // nowhere near, the line budget is.
    let controls: Vec<Value> = (1..=190)
        .map(|n| control(&format!("c{n}"), "button", "button", &format!("Go {n}")))
        .collect();
    let before = snapshot(
        "https://app.test/many",
        "Many",
        &"word ".repeat(600),
        controls,
    );
    let mut after = before.clone();
    after["controls"][0]["text"] = json!("Go first");
    extension.reply("page/snapshot", Ok(snapshot_reply(&before)));
    extension.reply(
        "page/click",
        Ok(observed_reply(json!({"ref": "c2"}), &after, 7)),
    );
    let registry = registry(&extension);
    call(&registry, "chrome_page_snapshot", json!({})).await;

    let click = call(&registry, "chrome_click", json!({"ref": "c2"})).await;

    let text = &click.text;
    assert!(
        text.lines().count() <= LINES,
        "{} lines; the runtime would keep the two ends of anything over 200",
        text.lines().count()
    );
    assert!(text.chars().count() <= CAP);
    assert!(
        text.lines().next().unwrap().contains("UNTRUSTED"),
        "the label leads:\n{text}"
    );
    assert_eq!(
        text.lines().nth(1),
        Some("Outcome: 1 control changed."),
        "the outcome is the first thing after the label:\n{text}"
    );
    let (listed, omitted) = controls_listed_and_omitted(text, "Go");
    assert!(omitted > 0, "{listed} listed");
    assert_eq!(listed + omitted, 190, "every control is listed or counted");
    assert!(
        listed >= 130,
        "the budget is spent on controls, not wasted: {listed} listed"
    );
    assert!(
        text.contains("c1 button \"Go first\" [changed]"),
        "the changed control is in what is listed:\n{text}"
    );
    assert!(
        text.contains("… text cut at 0 chars"),
        "the text went first, all of it:\n{}",
        &text[text.len().saturating_sub(300)..]
    );
    assert!(
        text.lines()
            .last()
            .unwrap()
            .starts_with("… text cut at 0 chars"),
        "the last word is the text marker, so nothing after it can be cut away"
    );
}

#[tokio::test]
async fn forms_and_frames_are_cut_before_the_controls_and_their_counts_are_shown() {
    let extension = Arc::new(Scripted::default());
    let controls: Vec<Value> = (1..=140)
        .map(|n| control(&format!("c{n}"), "button", "button", &format!("Go {n}")))
        .collect();
    let mut page = snapshot(
        "https://app.test/forms",
        "Forms",
        &"word ".repeat(300),
        controls,
    );
    page["forms"] = json!(
        (1..=12)
            .map(
                |n| json!({"id": format!("f{n}"), "method": "post", "action": "/save",
            "fields": [{"ref": format!("c{n}"), "label": "Name", "type": "text"}]})
            )
            .collect::<Vec<_>>()
    );
    page["iframes"] = json!((1..=8)
        .map(|n| json!({"index": n, "crossOrigin": false, "src": format!("https://app.test/{n}")}))
        .collect::<Vec<_>>());
    extension.reply("page/snapshot", Ok(snapshot_reply(&page)));
    let registry = registry(&extension);

    let looked = call(&registry, "chrome_page_snapshot", json!({})).await;

    let text = &looked.text;
    assert!(
        text.lines().count() <= LINES && text.chars().count() <= CAP,
        "{} lines, {} chars",
        text.lines().count(),
        text.chars().count()
    );
    let (listed, omitted) = controls_listed_and_omitted(text, "Go");
    assert_eq!(listed + omitted, 140, "every control is listed or counted");
    assert!(
        listed >= 120,
        "the controls keep most of the room: {listed}"
    );
    assert!(
        text.contains("… 12 more forms not listed") && text.contains("… 8 more frames not listed"),
        "forms and frames say how many were left out:\n{}",
        &text[text.len().saturating_sub(400)..]
    );
    assert!(
        at(text, "Controls") < at(text, "Forms:")
            && at(text, "Forms:") < at(text, "Frames:")
            && at(text, "Frames:") < at(text, "Text:"),
        "{text}"
    );
    assert!(
        text.lines()
            .last()
            .unwrap()
            .starts_with("… text cut at 0 chars"),
        "{}",
        &text[text.len().saturating_sub(300)..]
    );
}
