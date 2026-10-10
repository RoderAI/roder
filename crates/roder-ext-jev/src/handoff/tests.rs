use serde_json::{Value, json};

use super::*;

fn data(status: &str, url: &str, why: &str) -> Value {
    json!({"status": status, "url": url, "stopped_because": why})
}

/// What the session does with one finished call's data.
fn finish(last: &mut Option<Handoff>, mut data: Value) -> (bool, Option<String>) {
    classify(last, &mut data);
    (
        is_error(&data),
        data["outcome_class"].as_str().map(str::to_string),
    )
}

/// The longest run of errors, which is what Roder's reliability accounting
/// stops on at five.
fn longest_error_run(errors: &[bool]) -> usize {
    errors
        .iter()
        .fold((0, 0), |(longest, run), error| {
            let run = if *error { run + 1 } else { 0 };
            (longest.max(run), run)
        })
        .0
}

#[test]
fn a_first_handoff_is_marked_and_is_not_an_error() {
    for status in ["needs_input", "needs_confirmation", "access_denied"] {
        let mut last = None;
        let (error, class) = finish(&mut last, data(status, "https://a.test/", "why"));
        assert!(!error, "{status}");
        assert_eq!(class.as_deref(), Some("handoff"), "{status}");
        assert!(last.is_some());
    }
}

#[test]
fn five_different_handoffs_in_a_row_are_not_a_run_of_errors() {
    let mut last = None;
    let errors = [
        data("access_denied", "https://a.test/", "refused (HTTP 403)"),
        data(
            "needs_input",
            "https://b.test/form",
            "no value for \"Name\"",
        ),
        // The same page, another field.
        data(
            "needs_input",
            "https://b.test/form",
            "no value for \"Email\"",
        ),
        data(
            "needs_confirmation",
            "https://b.test/pay",
            "did not click Pay",
        ),
        // The same refusal, another site.
        data("access_denied", "https://c.test/", "refused (HTTP 403)"),
    ]
    .into_iter()
    .map(|data| finish(&mut last, data).0)
    .collect::<Vec<_>>();
    assert_eq!(errors, [false; 5]);
    assert_eq!(longest_error_run(&errors), 0);
}

#[test]
fn an_identical_handoff_is_a_repeat_and_an_error_for_as_long_as_it_repeats() {
    let mut last = None;
    let same = || {
        data(
            "needs_input",
            "https://a.test/form",
            "no value for \"Name\"",
        )
    };
    let seen = (0..6)
        .map(|_| finish(&mut last, same()))
        .collect::<Vec<_>>();
    assert_eq!(
        seen.iter().map(|(error, _)| *error).collect::<Vec<_>>(),
        [false, true, true, true, true, true]
    );
    assert_eq!(seen[0].1.as_deref(), Some("handoff"));
    assert!(
        seen[1..]
            .iter()
            .all(|(_, class)| class.as_deref() == Some("repeated_handoff"))
    );
    // The handoff after a repeat that differs is a first again.
    let (error, class) = finish(
        &mut last,
        data(
            "needs_input",
            "https://a.test/form",
            "no value for \"Email\"",
        ),
    );
    assert!(!error);
    assert_eq!(class.as_deref(), Some("handoff"));
}

#[test]
fn another_status_page_or_reason_is_not_a_repeat() {
    let base = (
        "needs_input",
        "https://a.test/form",
        "no value for \"Name\"",
    );
    for (status, url, why) in [
        ("needs_confirmation", base.1, base.2),
        (base.0, "https://a.test/other", base.2),
        (base.0, base.1, "no value for \"Email\""),
    ] {
        let mut last = None;
        finish(&mut last, data(base.0, base.1, base.2));
        let (error, class) = finish(&mut last, data(status, url, why));
        assert!(!error, "{status} {url} {why}");
        assert_eq!(class.as_deref(), Some("handoff"));
    }
}

#[test]
fn whitespace_in_the_reason_does_not_make_a_handoff_new() {
    let mut last = None;
    finish(
        &mut last,
        data(
            "access_denied",
            "https://a.test/",
            "The site refused\n  access",
        ),
    );
    let (error, class) = finish(
        &mut last,
        data(
            "access_denied",
            "https://a.test/",
            " The site  refused access ",
        ),
    );
    assert!(error);
    assert_eq!(class.as_deref(), Some("repeated_handoff"));
}

#[test]
fn any_other_result_in_between_starts_the_run_over() {
    for between in [
        data("done", "https://a.test/", ""),
        data("blocked", "https://a.test/", "stalled"),
        data("timed_out", "https://a.test/", "slow"),
        data("error", "https://a.test/", "broken"),
    ] {
        let mut last = None;
        let handoff = || data("access_denied", "https://a.test/", "refused");
        finish(&mut last, handoff());
        let status = between["status"].clone();
        let (_, class) = finish(&mut last, between);
        assert_eq!(class, None, "{status}");
        assert_eq!(last, None, "{status}");
        let (error, class) = finish(&mut last, handoff());
        assert!(!error, "{status}");
        assert_eq!(class.as_deref(), Some("handoff"), "{status}");
    }
}

#[test]
fn every_other_status_keeps_its_meaning_and_gains_nothing() {
    let mut last = None;
    for status in [
        "blocked",
        "budget_exceeded",
        "timed_out",
        "unavailable",
        "error",
        "busy",
        "ready",
        "something_new",
    ] {
        let (error, class) = finish(&mut last, data(status, "https://a.test/", "x"));
        assert!(error, "{status}");
        assert_eq!(class, None, "{status}");
    }
    for status in ["done", "closed"] {
        let (error, class) = finish(&mut last, data(status, "https://a.test/", ""));
        assert!(!error, "{status}");
        assert_eq!(class, None, "{status}");
    }
    // No status at all is not a success.
    assert!(is_error(&json!({})));
    assert!(is_error(&json!({"status": null})));
}

#[test]
fn a_handoff_status_the_session_did_not_mark_is_still_an_error() {
    for status in ["needs_input", "needs_confirmation", "access_denied"] {
        assert!(
            is_error(&data(status, "https://a.test/", "why")),
            "{status}"
        );
        let mut forged = data(status, "https://a.test/", "why");
        forged["outcome_class"] = json!("repeated_handoff");
        assert!(is_error(&forged), "{status}");
    }
}

#[test]
fn the_text_the_caller_reads_is_the_same_with_or_without_the_class() {
    for status in ["needs_input", "needs_confirmation", "access_denied"] {
        let mut plain = data(status, "https://a.test/form", "no value for \"Name\"");
        plain["actions"] = json!([]);
        plain["visible_text"] = json!("Contact us");
        let mut marked = plain.clone();
        classify(&mut None, &mut marked);
        assert_eq!(marked["outcome_class"], json!("handoff"));
        let mut repeated = plain.clone();
        let mut last = None;
        classify(&mut last, &mut repeated);
        classify(&mut last, &mut repeated);
        assert_eq!(repeated["outcome_class"], json!("repeated_handoff"));

        let text = crate::report::digest(&plain, "NOW");
        assert_eq!(crate::report::digest(&marked, "NOW"), text, "{status}");
        assert_eq!(crate::report::digest(&repeated, "NOW"), text, "{status}");
        assert!(!text.contains("handoff"), "{text}");
    }
}

#[test]
fn a_long_reason_that_comes_back_unchanged_still_repeats() {
    let long = |tail: &str| format!("{}{tail}", "same start ".repeat(40));
    let mut last = None;
    finish(
        &mut last,
        data("needs_input", "https://a.test/", &long("A")),
    );
    // Compared whole, not cut: the same long reason repeats.
    let (error, _) = finish(
        &mut last,
        data("needs_input", "https://a.test/", &long("A")),
    );
    assert!(error);
}

#[test]
fn reasons_that_differ_only_far_into_the_text_are_not_a_repeat() {
    // The same status, page and first 300 characters, then different words:
    // two handoffs, not the caller ignoring one. The second is a first.
    let long = |tail: &str| format!("{}{tail}", "x".repeat(300));
    let mut last = None;
    finish(
        &mut last,
        data("needs_confirmation", "https://a.test/", &long("A")),
    );
    let (error, class) = finish(
        &mut last,
        data("needs_confirmation", "https://a.test/", &long("B")),
    );
    assert!(!error);
    assert_eq!(class.as_deref(), Some("handoff"));
    // And the second one, repeated unchanged, is a repeat.
    let (error, class) = finish(
        &mut last,
        data("needs_confirmation", "https://a.test/", &long("B")),
    );
    assert!(error);
    assert_eq!(class.as_deref(), Some("repeated_handoff"));
}

#[test]
fn addresses_that_differ_only_far_into_the_text_are_not_a_repeat() {
    let long = |tail: &str| format!("https://a.test/?q={}{tail}", "x".repeat(2048));
    let mut last = None;
    finish(&mut last, data("needs_input", &long("1"), "no value"));
    let (error, class) = finish(&mut last, data("needs_input", &long("2"), "no value"));
    assert!(!error);
    assert_eq!(class.as_deref(), Some("handoff"));
    let (error, class) = finish(&mut last, data("needs_input", &long("2"), "no value"));
    assert!(error);
    assert_eq!(class.as_deref(), Some("repeated_handoff"));
}

#[test]
fn whitespace_far_into_a_long_reason_does_not_make_a_handoff_new() {
    let words = "same start ".repeat(80);
    let mut last = None;
    finish(
        &mut last,
        data("access_denied", "https://a.test/", &format!("{words}end")),
    );
    let (error, class) = finish(
        &mut last,
        data(
            "access_denied",
            "https://a.test/",
            &format!(" {}\n end ", words.replace(' ', "  ")),
        ),
    );
    assert!(error);
    assert_eq!(class.as_deref(), Some("repeated_handoff"));
}
