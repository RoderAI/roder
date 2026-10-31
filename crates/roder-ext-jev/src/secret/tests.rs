use serde_json::json;

use super::*;

#[test]
fn dom_scrubbing_handles_json_escaping_and_accessible_name_whitespace() {
    for secret in [
        "ab\"cd",
        "ab\\cd",
        "ab\ncd",
        "ab  cd",
        "ab\u{feff}cd",
        "ab\u{85}cd",
    ] {
        let mut secrets = Secrets::default();
        secrets.remember(secret);
        let normalized = secret
            .split(|c: char| (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}')
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let mut page = json!({"dom_snapshot":format!("- text {}\n- button {}\n- button Save", serde_json::to_string(secret).unwrap(), serde_json::to_string(&normalized).unwrap()),"actions":[]});
        secrets.scrub_observation(&mut page);
        assert_eq!(
            page["dom_snapshot"],
            "- text \"[secret]\"\n- button \"[secret]\"\n- button Save"
        );
    }
}

#[test]
fn short_secret_input_survives_session_transfer_without_storing_the_value() {
    let mut first = Secrets::default();
    first.remember("12");
    assert_eq!(first.len(), 0);
    assert!(first.has_input());
    let mut next = Secrets::default();
    next.extend(&first);
    assert!(next.clone().has_input());
    assert_eq!(next.len(), 0);
    let mut page = json!({"actions":[{"form":{"fields":["echo-long-secret"],"submit_buttons":["echo-long-secret"]}}],"disabled_controls":[{"label":"echo-long-secret"}]});
    next.remember("echo-long-secret");
    next.scrub_observation(&mut page);
    assert!(!page.to_string().contains("echo-long-secret"));
}

#[test]
fn only_passwords_and_one_time_codes_are_secret() {
    assert!(is_secret(&json!({"input_type": "password"})));
    assert!(is_secret(&json!({"input_type": "one-time-code"})));
    assert!(!is_secret(&json!({"input_type": "text"})));
    assert!(!is_secret(&json!({"kind": "fill"})));
}

#[test]
fn a_typed_secret_is_scrubbed_from_what_leaves_the_loop() {
    let mut secrets = Secrets::default();
    secrets.remember("hunter2-7431");
    secrets.remember("12");
    let mut page = json!({
        "url": "https://x.test/login?password=hunter2-7431",
        "title": "Hi",
        "text": "Wrong password hunter2-7431 for 12 users",
        "marker": ["hunter2-7431"],
        "actions": [
            {"kind": "fill", "label": "Shown hunter2-7431", "value": "hunter2-7431"},
            {"kind": "select", "label": "x → hunter2-7431", "value": "hunter2-7431"},
            {"kind": "click", "label": "Continue", "section": "Code hunter2-7431 sent"},
        ],
        "dialogs": [{"type": "alert", "message": "hunter2-7431?", "accepted": true}],
        "frames": [{"origin": "https://accounts.test", "text": "Code hunter2-7431 confirmed"}],
    });
    secrets.scrub_observation(&mut page);
    assert_eq!(page["url"], "https://x.test/login?password=[secret]");
    // A two-character secret is not scrubbed.
    assert_eq!(page["text"], "Wrong password [secret] for 12 users");
    assert_eq!(page["actions"][0]["value"], "[secret]");
    assert_eq!(page["actions"][1]["label"], "x → [secret]");
    assert_eq!(page["actions"][1]["value"], "hunter2-7431");
    assert_eq!(page["dialogs"][0]["message"], "[secret]?");
    // The heading a control sits under, and a frame showing it back.
    assert_eq!(page["actions"][2]["section"], "Code [secret] sent");
    assert_eq!(page["frames"][0]["text"], "Code [secret] confirmed");
    // Compared with the live page, never reported.
    assert_eq!(page["marker"][0], "hunter2-7431");
}

#[test]
fn the_field_values_the_text_lists_are_scrubbed_with_it() {
    let mut secrets = Secrets::default();
    secrets.remember("hunter2-7431");
    let mut page = json!({
        "text": "Note\nhunter2-7431 here\nSave",
        "typed_values": ["hunter2-7431 here", "plain"],
    });
    secrets.scrub_observation(&mut page);
    // Still the same lines, so the page text can be told from the fields'.
    assert_eq!(page["text"], "Note\n[secret] here\nSave");
    assert_eq!(page["typed_values"], json!(["[secret] here", "plain"]));
}

/// A GET form, `encodeURIComponent`, `URLSearchParams` and a JSON body each
/// carry `@`, a space and `&` differently; every one is caught.
#[test]
fn a_secret_is_caught_in_every_encoding_an_address_or_a_body_carries_it_in() {
    let mut secrets = Secrets::default();
    secrets.remember("p@ss word&7431é\"");
    for (carried, expected) in [
        ("raw p@ss word&7431é\" end", "raw [secret] end"),
        (
            "/landing?password=p%40ss+word%267431%C3%A9%22&next=1",
            "/landing?password=[secret]&next=1",
        ),
        ("/x?p=p%40ss%20word%267431%C3%A9%22", "/x?p=[secret]"),
        (
            r#"{"password":"p@ss word&7431é\""}"#,
            r#"{"password":"[secret]"}"#,
        ),
        (
            r#"{"password":"p@ss word&7431é\""}"#,
            r#"{"password":"[secret]"}"#,
        ),
    ] {
        assert_eq!(secrets.scrub(carried), expected, "{carried}");
    }
    // `encodeURIComponent` keeps `'` and `(`, which a form submission encodes.
    let mut quoted = Secrets::default();
    quoted.remember("it's (mine)");
    assert_eq!(quoted.scrub("q=it's%20(mine)"), "q=[secret]");
    assert_eq!(quoted.scrub("q=it%27s+%28mine%29"), "q=[secret]");
}

#[test]
fn scrubbing_twice_changes_nothing() {
    let mut secrets = Secrets::default();
    // A value inside the marker itself, and one that overlaps another.
    secrets.remember("secret");
    secrets.remember("cret]x");
    let once = secrets.scrub("a secret then [secret] and cret]x");
    assert_eq!(once, "a [secret] then [secret] and [secret]");
    assert_eq!(secrets.scrub(&once), once);
}

#[test]
fn overlapping_secrets_become_one_marker() {
    let mut secrets = Secrets::default();
    secrets.remember("abcdef");
    secrets.remember("defghi");
    assert_eq!(secrets.scrub("xabcdefghiy"), "x[secret]y");
}

/// A password that is an ordinary word stays in the host of the site it
/// was typed on, and is scrubbed from its path, its query, its userinfo and
/// any other host.
#[test]
fn the_host_a_secret_was_typed_on_keeps_its_name() {
    let mut secrets = Secrets::default();
    secrets.remember_at("practice", Some("https://practice.expandtesting.com/login"));
    assert_eq!(
        secrets.scrub("Go to https://practice.expandtesting.com/secure?pw=practice."),
        "Go to https://practice.expandtesting.com/secure?pw=[secret]."
    );
    assert_eq!(
        secrets.scrub("https://practice@practice.expandtesting.com/"),
        "https://[secret]@practice.expandtesting.com/"
    );
    assert_eq!(
        secrets.scrub("https://practice.other.test/ and practice"),
        "https://[secret].other.test/ and [secret]"
    );
    // Typed nowhere known: no host is spared.
    let mut anywhere = Secrets::default();
    anywhere.remember("practice");
    assert_eq!(
        anywhere.scrub("https://practice.expandtesting.com/"),
        "https://[secret].expandtesting.com/"
    );
    // The origin a session hands on is kept with the value.
    let mut next = Secrets::default();
    next.extend(&secrets);
    assert_eq!(
        next.scrub("https://practice.expandtesting.com/"),
        "https://practice.expandtesting.com/"
    );
}
