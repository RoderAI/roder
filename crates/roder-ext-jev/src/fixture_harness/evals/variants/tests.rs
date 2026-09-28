use super::*;
use crate::decide::request_body;
use crate::prompts::NEXT_ACTION;

fn page() -> Value {
    json!({
        "url": "http://x/", "title": "Shop", "text": "Shop",
        "actions": [
            {"id": "e1", "node": 1, "kind": "click", "label": "Add to cart", "context": "Desk Lamp"},
            {"id": "e2", "node": 2, "kind": "click", "label": "Add to cart", "context": "Yoga Mat"},
            {"id": "e3", "node": 3, "kind": "click", "label": "Place order", "offscreen": true},
            {"id": "e4", "node": 4, "kind": "fill", "label": "Coupon"},
            {"id": "wait", "kind": "wait", "label": "Wait for the page to update"},
        ],
    })
}

fn rewritten(names: &str) -> (Value, Added) {
    let (mut body, _, _) = request_body(
        &page(),
        "Buy the lamp",
        &[],
        "jev-latest",
        chrono::NaiveDate::default(),
    );
    let added = rewrite_request(&mut body, &Variants::parse(names).unwrap());
    (body, added)
}

fn sentences(text: &str) -> Vec<String> {
    let mut out = text
        .replace('\n', " ")
        .split(". ")
        .map(|sentence| sentence.trim().trim_end_matches('.').to_string())
        .collect::<Vec<_>>();
    out.sort();
    out
}

#[test]
fn variant_names_parse_and_unknown_names_fail() {
    assert_eq!(Variants::parse("").unwrap(), Variants::default());
    let all = Variant::ALL.map(Variant::name).join(",");
    assert_eq!(Variants::parse(&all).unwrap().names().len(), 7);
    assert!(Variants::parse("no_context,bogus").is_err());
}

#[test]
fn the_baseline_request_is_untouched() {
    let (before, _, _) = request_body(
        &page(),
        "Buy the lamp",
        &[],
        "jev-latest",
        chrono::NaiveDate::default(),
    );
    let (after, added) = rewritten("");
    assert_eq!(before, after);
    assert!(added.nouls.is_empty());
}

#[test]
fn keyed_rules_carry_every_next_action_sentence() {
    let mut original = sentences(NEXT_ACTION);
    let first = "Advance the user's entire goal from the CURRENT page using one operation";
    assert!(original.contains(&first.to_string()));
    original.retain(|sentence| sentence != first);
    let keyed = KEYED_RULES
        .iter()
        .filter(|(key, _)| *key != "question")
        .map(|(_, rule)| *rule)
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(sentences(&keyed), original);
}

#[test]
fn no_context_strips_context_and_offscreen_everywhere() {
    let (body, _) = rewritten("no_context");
    let text = body.to_string();
    // The key, not TARGET's sentence that mentions offscreen elements.
    assert!(
        !text.contains("Desk Lamp") && !text.contains("\"offscreen\""),
        "{text}"
    );
    let (baseline, _) = rewritten("");
    let baseline = baseline.to_string();
    assert!(baseline.contains("Desk Lamp") && baseline.contains("\"offscreen\""));
}

#[test]
fn goal_in_state_sends_the_goal_once() {
    let (body, _) = rewritten("goal_in_state");
    assert_eq!(body["state"]["task"]["goal"], json!("Buy the lamp"));
    assert_eq!(body.to_string().matches("Buy the lamp").count(), 1);
    let operation = &body["questions"]["operation"]["instructions"];
    assert_eq!(operation["goal"], json!("`task.goal`"));
    assert!(operation["rules"]["completion"].is_string());
    let click = &body["questions"]["click_target"]["instructions"];
    assert_eq!(click["operation"], json!("CLICK"));
    assert_eq!(click["rules"]["target"], json!(TARGET));
}

#[test]
fn structured_criteria_give_every_operation_what_and_not_for() {
    let (body, _) = rewritten("structured_criteria");
    let criteria = body["questions"]["operation"]["criteria"]
        .as_object()
        .unwrap();
    assert!(criteria.contains_key("WAIT") && criteria.contains_key("DONE"));
    for (operation, entry) in criteria {
        assert!(entry["what"].is_string(), "{operation}");
        assert!(entry["not_for"].is_string(), "{operation}");
    }
}

#[test]
fn shadow_nouls_are_asked_recorded_and_stripped() {
    let (body, added) = rewritten("handoff_nouls,irreversible_nouls");
    let questions = body["questions"].as_object().unwrap();
    // Only "Place order" is on the production gate's shortlist.
    assert_eq!(
        added.nouls,
        vec![
            "goal_satisfied",
            "needs_user_info",
            "access_wall",
            "irreversible_click_3",
            "goal_authorizes_commitment"
        ]
    );
    for key in &added.nouls {
        assert_eq!(questions[key]["type"], json!("noul"), "{key}");
    }
    let mut response = json!({"answers": {
        "operation": {"choice": "CLICK", "probabilities": {}, "confidence": 0.9},
        "goal_satisfied": {"type": "noul", "noul": 0.2},
        "irreversible_click_3": {"type": "noul", "noul": 0.95},
    }});
    let variants = Variants::parse("handoff_nouls,irreversible_nouls").unwrap();
    let telemetry = read_response(&mut response, &variants, &added, 10);
    assert_eq!(telemetry.nouls["goal_satisfied"], 0.2);
    assert_eq!(telemetry.nouls["irreversible_click_3"], 0.95);
    // The same question the production gate asks.
    let (mut gated, _, _) = request_body(
        &page(),
        "Buy the lamp",
        &[],
        "jev-latest",
        chrono::NaiveDate::default(),
    );
    crate::irreversible::add_questions(&mut gated);
    assert_eq!(
        questions["irreversible_click_3"],
        gated["questions"]["irreversible_click_3"]
    );
    assert!(response["answers"].get("goal_satisfied").is_none());
    assert_eq!(telemetry.operation.as_deref(), Some("CLICK"));
}

#[test]
fn a_winning_none_is_recorded_and_falls_back_to_the_best_target() {
    let (body, added) = rewritten("none_target");
    assert_eq!(
        body["questions"]["click_target"]["criteria"]["none"],
        json!(NONE_OPTION)
    );
    assert_eq!(
        body["questions"]["type_text_target"]["criteria"]["none"],
        json!(NONE_OPTION)
    );
    let mut response = json!({"answers": {
        "operation": {"choice": "CLICK", "probabilities": {}, "confidence": 0.9},
        "click_target": {"choice": "none", "confidence": 0.5,
            "probabilities": {"1": 0.1, "2": 0.3, "3": 0.1, "none": 0.5}},
        "type_text_target": {"choice": "4", "confidence": 0.8,
            "probabilities": {"4": 0.8, "none": 0.2}},
    }});
    let variants = Variants::parse("none_target").unwrap();
    let telemetry = read_response(&mut response, &variants, &added, 10);
    assert!(telemetry.none_won);
    assert_eq!(telemetry.none_heads, vec!["click_target"]);
    let click = &response["answers"]["click_target"];
    assert_eq!(click["choice"], json!("2"));
    let total = click["probabilities"]
        .as_object()
        .unwrap()
        .values()
        .map(|value| value.as_f64().unwrap())
        .sum::<f64>();
    assert!((total - 1.0).abs() < 1e-9);
    assert!(click["probabilities"].get("none").is_none());
    assert_eq!(
        response["answers"]["type_text_target"]["choice"],
        json!("4")
    );
}

#[test]
fn a_request_the_gate_already_asked_is_not_asked_twice() {
    let (mut body, _, _) = request_body(
        &page(),
        "Buy the lamp",
        &[],
        "jev-latest",
        chrono::NaiveDate::default(),
    );
    crate::irreversible::add_questions(&mut body);
    let before = body.clone();
    let added = rewrite_request(&mut body, &Variants::parse("irreversible_nouls").unwrap());
    assert!(added.nouls.is_empty());
    assert_eq!(body, before);
    // So its answers are left for the gate to read.
    let mut response = json!({"answers": {
        "operation": {"choice": "CLICK", "probabilities": {}, "confidence": 0.9},
        "irreversible_click_3": {"type": "noul", "noul": 0.95},
    }});
    let variants = Variants::parse("irreversible_nouls").unwrap();
    read_response(&mut response, &variants, &added, 10);
    assert!(response["answers"].get("irreversible_click_3").is_some());
}
