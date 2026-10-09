// `chrome_select` on Roder Desktop's browser: chosen by ref and by selector,
// the value before the text, a miss that lists the options, the targets it
// refuses, and a page that puts the old value back. Also the two tools the
// Desktop browser has no route for, which must say so.
async fn desktop_select(registry: &ToolRegistry, url: &str) {
    let opened = call(
        registry,
        "chrome_navigate",
        json!({"url": format!("{url}select")}),
    )
    .await;
    assert!(!opened.is_error, "{}", opened.text);
    let looked = call(registry, "chrome_page_snapshot", json!({})).await;
    let ref_of = |label: &str| {
        looked.data["page"]["elements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["tag"] == "select" && e["label"] == label)
            .unwrap_or_else(|| panic!("no select labelled {label}: {}", looked.text))["ref"]
            .clone()
    };
    let value_of = |id: &str| format!("document.querySelector('#{id}').value");

    // 1. By ref, by the option's value: one change event, shown in the result.
    let chosen = call(
        registry,
        "chrome_select",
        json!({"ref": ref_of("Plan"), "value": "growth"}),
    )
    .await;
    assert!(!chosen.is_error, "{}", chosen.text);
    assert!(
        chosen.text.contains("Chose \"Growth plan\" (value growth)"),
        "{}",
        chosen.text
    );
    assert_eq!(eval(registry, &value_of("plan")).await, "growth");
    assert_eq!(
        eval(
            registry,
            "window.changes.filter(c => c[0] === 'plan').length"
        )
        .await,
        1
    );
    assert_eq!(chosen.data["events_trusted"], json!(false));
    assert_eq!(chosen.data["chosen"]["value"], "growth");

    // 2. By selector, by the option's visible text.
    let chosen = call(
        registry,
        "chrome_select",
        json!({"selector": "#plan", "value": "Pro"}),
    )
    .await;
    assert!(!chosen.is_error, "{}", chosen.text);
    assert_eq!(eval(registry, &value_of("plan")).await, "pro");

    // 3. The value comes before the text: "a" is the second option's value
    // and the first option's text, and "A" is only text.
    let by_value = call(
        registry,
        "chrome_select",
        json!({"selector": "#tier", "value": "a"}),
    )
    .await;
    assert!(!by_value.is_error, "{}", by_value.text);
    assert_eq!(eval(registry, &value_of("tier")).await, "a");
    let by_text = call(
        registry,
        "chrome_select",
        json!({"selector": "#tier", "value": "A"}),
    )
    .await;
    assert!(!by_text.is_error, "{}", by_text.text);
    assert_eq!(eval(registry, &value_of("tier")).await, "b");

    // 4. A miss lists the options (values where they differ from the text,
    // disabled ones marked) and changes nothing.
    let missed = call(
        registry,
        "chrome_select",
        json!({"selector": "#plan", "value": "Enterprise"}),
    )
    .await;
    assert!(missed.is_error, "{}", missed.text);
    for listed in [
        "\"Starter\"",
        "\"Growth plan\" (value growth)",
        "\"Pro\"",
        "\"Legacy\" (disabled)",
    ] {
        assert!(missed.text.contains(listed), "{listed}: {}", missed.text);
    }
    assert_eq!(eval(registry, &value_of("plan")).await, "pro");
    // The list is capped: 20 of 30 options, and the rest are counted.
    let missed = call(
        registry,
        "chrome_select",
        json!({"selector": "#country", "value": "Atlantis"}),
    )
    .await;
    assert!(missed.is_error, "{}", missed.text);
    assert!(missed.text.contains("\"Country 20\""), "{}", missed.text);
    assert!(!missed.text.contains("\"Country 21\""), "{}", missed.text);
    assert!(missed.text.contains("… and 10 more"), "{}", missed.text);
    // A disabled option and a text two options share are named, not chosen.
    let disabled = call(
        registry,
        "chrome_select",
        json!({"selector": "#plan", "value": "legacy"}),
    )
    .await;
    assert!(
        disabled.is_error && disabled.text.contains("\"Legacy\" is disabled"),
        "{}",
        disabled.text
    );
    let ambiguous = call(
        registry,
        "chrome_select",
        json!({"selector": "#dup", "value": "Same"}),
    )
    .await;
    assert!(
        ambiguous.is_error
            && ambiguous.text.contains("more than one option")
            && ambiguous.text.contains("(value 1)")
            && ambiguous.text.contains("(value 2)"),
        "{}",
        ambiguous.text
    );
    assert_eq!(eval(registry, &value_of("plan")).await, "pro");
    assert_eq!(eval(registry, &value_of("dup")).await, "1");

    // 5. Something that is not a select is refused, and not clicked.
    let button = call(
        registry,
        "chrome_select",
        json!({"selector": "#go", "value": "x"}),
    )
    .await;
    assert!(
        button.is_error
            && button.text.contains("button \"Go\"")
            && button.text.contains("not a native <select>"),
        "{}",
        button.text
    );
    assert_eq!(eval(registry, "window.goClicks").await, 0);

    // 6. Calls without exactly one target, or without a value, or aimed at
    // nothing, are errors that change nothing.
    for (args, wanted) in [
        (json!({"value": "growth"}), "needs a target"),
        (
            json!({"ref": ref_of("Plan"), "selector": "#plan", "value": "growth"}),
            "not both",
        ),
        (json!({"selector": "#plan"}), "needs value"),
        (
            json!({"selector": "#absent", "value": "x"}),
            "No visible target",
        ),
        (json!({"selector": "select", "value": "x"}), "ambiguous"),
        (json!({"ref": "e0-0", "value": "growth"}), "gone"),
    ] {
        let refused = call(registry, "chrome_select", args.clone()).await;
        assert!(
            refused.is_error && refused.text.contains(wanted),
            "{args}: {}",
            refused.text
        );
    }
    assert_eq!(eval(registry, &value_of("plan")).await, "pro");
    assert_eq!(
        eval(registry, "window.changes.length").await,
        4,
        "only the four successful choices may have fired a change"
    );

    // 7. A page that puts the old value back, at once or a moment later, is
    // reported as having done so, not as a success.
    for (selector, label) in [("#snap", "Snaps back"), ("#sticky", "Slips back")] {
        let restored = call(
            registry,
            "chrome_select",
            json!({"selector": selector, "value": "paid"}),
        )
        .await;
        assert!(
            restored.is_error
                && restored.text.contains("changed it back to \"Free\"")
                && restored.text.contains("did not stick"),
            "{label}: {}",
            restored.text
        );
        assert_eq!(restored.data["restored"], json!(true), "{label}");
        assert_eq!(
            eval(registry, &value_of(&selector[1..])).await,
            "free",
            "{label}"
        );
    }

    // 8. The tools the Desktop browser has no route for say so, instead of
    // claiming that no browser is connected.
    for (name, args, wanted) in [
        (
            "chrome_page_text",
            json!({"selector": "#log"}),
            "chrome_page_text",
        ),
        (
            "chrome_highlight",
            json!({"selector": "#log"}),
            "chrome_highlight",
        ),
    ] {
        let unsupported = call(registry, name, args).await;
        assert!(
            unsupported.is_error
                && unsupported.text.contains(wanted)
                && unsupported
                    .text
                    .contains("not supported on the Roder Desktop browser")
                && !unsupported.text.contains("is connected"),
            "{}",
            unsupported.text
        );
    }

    // 9. With no Desktop browser at all, "not connected" is still what a
    // tool with no Desktop route says.
    let live = std::env::var("RODER_DESKTOP_CDP_PORT").unwrap();
    let dead = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    unsafe {
        std::env::set_var("RODER_DESKTOP_CDP_PORT", dead.to_string());
    }
    let nowhere = call(registry, "chrome_page_text", json!({})).await;
    unsafe {
        std::env::set_var("RODER_DESKTOP_CDP_PORT", live);
    }
    assert!(
        nowhere.is_error && nowhere.text.contains("is connected"),
        "{}",
        nowhere.text
    );

    // 10. A select whose change loads another page: the choice was made, the
    // select can no longer be checked, and the page it left for is shown.
    let left = call(
        registry,
        "chrome_select",
        json!({"selector": "#jump", "value": "leave"}),
    )
    .await;
    assert!(
        !left.is_error
            && left.text.contains("Chose \"Leave\"")
            && left.text.contains("no longer on the page")
            && left.text.contains("Primitive fixture"),
        "{}",
        left.text
    );
}
