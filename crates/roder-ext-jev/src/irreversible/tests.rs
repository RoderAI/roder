//! Unit tests of the gate's shortlist, answers and verdicts.

use super::*;

#[test]
fn commitment_words_match_whole_words_in_any_case() {
    for label in [
        "Pay now",
        "PAY",
        "Payment details",
        "Place your order",
        "Delete account",
        "Send message",
        "Unsubscribe",
        "Confirm & book",
        "Submit application",
    ] {
        assert!(names_commitment(label), "{label}");
    }
    for label in [
        "Add to cart",
        "Display options",
        "Sign in",
        "Orders",
        "Paypal",
        "Next page",
        "",
    ] {
        assert!(!names_commitment(label), "{label}");
    }
}

#[test]
fn the_gate_applies_to_every_enter_and_to_committing_clicks_only() {
    let action = |kind: &str, label: &str| json!({"kind": kind, "label": label});
    assert!(gated(&action("click", "Pay now")));
    assert!(gated(&action("enter", "Message")));
    assert!(!gated(&action("click", "Add to cart")));
    // Typing and choosing an option commit nothing by themselves.
    assert!(!gated(&action("fill", "Order note")));
    assert!(!gated(&action("select", "Delivery → Express")));
    assert!(!gated(&action("wait", "Wait for the page to update")));
}

#[test]
fn a_noul_answer_is_validated_as_strictly_as_a_choice() {
    assert_eq!(
        noul_probability(&json!({"type": "noul", "noul": 0.93})),
        Some(0.93)
    );
    assert_eq!(noul_probability(&json!({"noul": 0})), Some(0.0));
    assert_eq!(noul_probability(&json!({"noul": 1})), Some(1.0));
    for invalid in [
        json!(null),
        json!(0.9),
        json!({}),
        json!({"noul": null}),
        json!({"noul": "0.9"}),
        json!({"noul": 1.2}),
        json!({"noul": -0.1}),
        json!({"type": "choice", "noul": 0.2}),
        json!({"type": "boolean", "probability": 0.2}),
    ] {
        assert_eq!(noul_probability(&invalid), None, "{invalid}");
    }
}

#[test]
fn only_the_chosen_actions_question_is_read() {
    let asked = vec![
        Asked {
            key: "irreversible_enter_4".into(),
            operation: "PRESS_ENTER".into(),
            target: "4".into(),
        },
        Asked {
            key: "irreversible_click_2".into(),
            operation: "CLICK".into(),
            target: "2".into(),
        },
    ];
    let answers = json!({
        "irreversible_enter_4": {"type": "noul", "noul": 0.1},
        "irreversible_click_2": {"type": "noul", "noul": 0.97},
    });
    assert_eq!(
        chosen_probability(&answers, &asked, "CLICK", Some("2")),
        Some(0.97)
    );
    assert_eq!(
        chosen_probability(&answers, &asked, "PRESS_ENTER", Some("4")),
        Some(0.1)
    );
    // Not asked, or asked about another operation's target.
    assert_eq!(
        chosen_probability(&answers, &asked, "CLICK", Some("4")),
        None
    );
    assert_eq!(chosen_probability(&answers, &asked, "DONE", None), None);
    // Asked, but not answered.
    assert_eq!(
        chosen_probability(&json!({}), &asked, "CLICK", Some("2")),
        None
    );
}

#[test]
fn an_unauthorized_run_stops_above_the_threshold_and_on_no_answer() {
    let pay = json!({"kind": "click", "label": "Pay now"});
    assert_eq!(verdict(&pay, Some(0.2), false, 1.0), Verdict::Dispatch);
    // At the threshold is not above it.
    assert_eq!(verdict(&pay, Some(0.5), false, 1.0), Verdict::Dispatch);
    let Verdict::Confirm(reason) = verdict(&pay, Some(0.93), false, 1.0) else {
        panic!("dispatched");
    };
    assert_eq!(
        reason,
        "Jev did not click \"Pay now\": it may make a purchase, payment, send, publish, delete \
         or other change that cannot be undone (P=0.93), and this run is not authorized to"
    );
    // The gate fails closed.
    let Verdict::Confirm(reason) = verdict(&pay, None, false, 1.0) else {
        panic!("dispatched without an answer");
    };
    assert!(
        reason.contains("(the model gave no valid answer about it)"),
        "{reason}"
    );
}

#[test]
fn an_authorized_run_still_needs_a_confident_decision() {
    let delete = json!({"kind": "click", "label": "Delete account"});
    assert_eq!(verdict(&delete, Some(0.99), true, 0.95), Verdict::Dispatch);
    assert_eq!(verdict(&delete, None, true, 0.90), Verdict::Dispatch);
    let Verdict::Confirm(reason) = verdict(&delete, Some(0.99), true, 0.72) else {
        panic!("an unsure authorized decision dispatched");
    };
    assert!(reason.contains("(0.72 < 0.90)"), "{reason}");
    assert!(reason.contains("not confident enough"), "{reason}");
    let enter = json!({"kind": "enter", "label": "Message"});
    let Verdict::Confirm(reason) = verdict(&enter, Some(0.8), false, 1.0) else {
        panic!("dispatched");
    };
    assert!(
        reason.starts_with("Jev did not press Enter in \"Message\""),
        "{reason}"
    );
}

fn body_with_clicks(labels: &[&str]) -> Value {
    let criteria = labels
        .iter()
        .enumerate()
        .map(|(n, label)| {
            let index = (n + 1).to_string();
            (
                index.clone(),
                json!({"element": format!("[{index}] {label}"), "current_value": ""}),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    json!({"questions": {
        "operation": {"type": "choice"},
        "click_target": {"type": "choice", "criteria": criteria},
    }})
}

#[test]
fn the_shortlist_is_capped_and_a_request_without_one_is_untouched() {
    let labels = (0..12).map(|_| "Delete").collect::<Vec<_>>();
    let mut body = body_with_clicks(&labels);
    let asked = add_questions(&mut body);
    assert_eq!(asked.len(), MAX_QUESTIONS);
    assert_eq!(asked[0].key, "irreversible_click_1");
    assert_eq!(asked[7].key, "irreversible_click_8");
    assert!(body["questions"].get("irreversible_click_9").is_none());

    let before = body_with_clicks(&["Add to cart", "Next page"]);
    let mut after = before.clone();
    assert!(add_questions(&mut after).is_empty());
    assert_eq!(
        serde_json::to_string(&after).unwrap(),
        serde_json::to_string(&before).unwrap()
    );
}

#[test]
fn a_label_holding_brackets_is_read_after_its_index() {
    let mut body = body_with_clicks(&["[beta] Pay now"]);
    let asked = add_questions(&mut body);
    assert_eq!(asked.len(), 1);
    assert!(is_gate_key(&asked[0].key));
    assert!(!is_gate_key("goal_satisfied"));
}
