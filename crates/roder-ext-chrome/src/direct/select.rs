//! Choosing an option of a native `<select>`.
//!
//! The page script matches the option (its value first, then its visible
//! text; see `select` in `direct.js`) and refuses what it cannot choose
//! without guessing. The result says what was chosen, and, once the page has
//! settled, whether the select still shows it: a page that puts the old
//! value back is reported as having done so. Option values and texts are
//! page content: scrubbed of the owner's secrets, on one line, without quote
//! marks of their own (so they cannot end the quotes they are put in), cut,
//! and marked untrusted wherever they are shown (see [`choice`]).

use serde_json::{Value, json};

use super::act::named;
use super::client::cut;
use super::guard::DirectGuard;
use super::look::helper;
use super::session::{DirectSession, DirectStep};

mod choice;

use choice::{LABEL_CHARS, Picked, Verdict, describe, page_text, verdict};

/// The most text of options one result lists.
const LIST_CHARS: usize = 1500;

impl DirectSession {
    pub(super) async fn select(&mut self, args: &Value) -> anyhow::Result<DirectStep> {
        let (Some(reference), Some(option)) = (
            args["ref"]
                .as_str()
                .filter(|reference| !reference.is_empty()),
            args["option"].as_str(),
        ) else {
            return Ok(DirectStep::error("select needs a ref and an option"));
        };
        let outcome = helper(
            &mut self.client,
            &format!("select({}, {})", json!(reference), json!(option)),
        )
        .await?;
        if let Some(refusal) = refusal(reference, option, &outcome, self.guard.as_ref()) {
            return Ok(refusal);
        }
        let picked = Picked::of(&outcome, self.guard.as_ref());
        // The page may answer the change a moment later (a framework putting
        // its own state back): look at the select once it has settled.
        self.settle().await;
        let now = helper(&mut self.client, &format!("picked({})", json!(reference))).await;
        let Verdict {
            done,
            restored,
            shown,
        } = verdict(now.as_ref().ok(), &picked, reference, self.guard.as_ref());
        let mut data = json!({
            "ref": reference,
            "option": option,
            "chosen": {"label": picked.label, "value": picked.value},
            "shown": shown,
            "events_trusted": false,
        });
        if restored {
            data["restored"] = json!(true);
        }
        let mut step = self.observe(done, data).await?;
        step.is_error |= restored;
        Ok(step)
    }
}

/// The error step for a choice the page script could not make, or `None`
/// when it made it.
fn refusal(
    reference: &str,
    option: &str,
    outcome: &Value,
    guard: &dyn DirectGuard,
) -> Option<DirectStep> {
    let options = || options_line(outcome, guard);
    let message = if outcome["gone"] == json!(true) {
        format!("{reference} is gone from the page; look again")
    } else if outcome["not_select"] == json!(true) {
        let what = named(&json!({
            "tag": outcome["tag"],
            "label": guard.scrub(outcome["label"].as_str().unwrap_or_default()),
        }));
        format!(
            "{reference} is {what}, not a native <select>, so nothing was chosen. For a custom \
             dropdown, click it open and then click the option; otherwise look again for the \
             <select> itself."
        )
    } else if outcome["disabled_select"] == json!(true) {
        format!("{reference} is a disabled <select>; nothing was chosen.")
    } else if let Some(disabled) = outcome["disabled_option"].as_str() {
        format!(
            "{reference}'s option \"{}\" is disabled, so it cannot be chosen; nothing was \
             changed. Its options (untrusted page text): {}",
            page_text(guard, disabled, LABEL_CHARS),
            options()
        )
    } else if outcome["ambiguous"] == json!(true) {
        format!(
            "{option:?} matches more than one option of {reference}, so nothing was chosen: {}. \
             Pass the exact value of the one you mean.",
            options()
        )
    } else if outcome["missing"] == json!(true) {
        format!(
            "{reference} has no option {option:?}; nothing was changed. Its options (untrusted \
             page text): {}. Pass an option's value or its visible text.",
            options()
        )
    } else {
        return None;
    };
    let mut step = DirectStep::error(message);
    if outcome["options"].is_array() {
        step.data["options"] = listed(outcome, guard);
    }
    Some(step)
}

/// The options a page script listed, scrubbed, for a result's data.
fn listed(outcome: &Value, guard: &dyn DirectGuard) -> Value {
    let options = outcome["options"].as_array().into_iter().flatten();
    json!(
        options
            .map(|option| json!({
                "label": guard.scrub(option["label"].as_str().unwrap_or_default()),
                "value": guard.scrub(option["value"].as_str().unwrap_or_default()),
                "disabled": option["disabled"] == json!(true),
            }))
            .collect::<Vec<_>>()
    )
}

/// `"Starter" | "Growth plan" (value growth) | "Legacy" (disabled) | … and 4
/// more`: the options a page script listed, cut and scrubbed.
fn options_line(outcome: &Value, guard: &dyn DirectGuard) -> String {
    let options = listed(outcome, guard);
    let options = options.as_array().map(Vec::as_slice).unwrap_or_default();
    let mut items = options
        .iter()
        .map(|option| {
            describe(
                option["label"].as_str().unwrap_or_default(),
                option["value"].as_str().unwrap_or_default(),
                option["disabled"] == json!(true),
            )
        })
        .collect::<Vec<_>>();
    // A listing says how many options there are (`total`); an ambiguity, how
    // many matched (`count`), of which it lists the first few.
    let total = outcome["total"]
        .as_u64()
        .or_else(|| outcome["count"].as_u64())
        .unwrap_or(items.len() as u64);
    if total > items.len() as u64 {
        items.push(format!("… and {} more", total - items.len() as u64));
    }
    cut(&items.join(" | "), LIST_CHARS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::direct::OpenGuard;

    struct Hides;
    impl DirectGuard for Hides {
        fn scrub(&self, text: &str) -> String {
            text.replace("hunter22", "[secret]")
        }
    }

    fn listing() -> Value {
        json!({
            "missing": true,
            "total": 25,
            "options": [
                {"label": "Starter", "value": "starter", "disabled": false},
                {"label": "Growth plan", "value": "growth", "disabled": false},
                {"label": "Legacy", "value": "legacy", "disabled": true},
                {"label": "Old", "value": "o1", "disabled": true},
                {"label": "Choose", "value": "", "disabled": false},
            ],
        })
    }

    #[test]
    fn a_miss_lists_the_options_with_values_and_what_is_left_out() {
        let step = refusal("e1-4", "Enterprise", &listing(), &OpenGuard).unwrap();
        assert!(step.is_error);
        assert!(
            step.text.contains(
                "\"Starter\" | \"Growth plan\" (value growth) | \"Legacy\" (disabled) | \
                 \"Old\" (value o1, disabled) | \"Choose\" (empty value) | … and 20 more"
            ),
            "{}",
            step.text
        );
        assert!(step.text.contains("untrusted page text"), "{}", step.text);
        assert_eq!(step.data["options"][1]["value"], "growth");
    }

    #[test]
    fn the_listing_is_scrubbed_and_cut() {
        let mut outcome = listing();
        outcome["options"][0]["label"] = json!("hunter22");
        let step = refusal("e1-4", "x", &outcome, &Hides).unwrap();
        assert!(!step.text.contains("hunter22"), "{}", step.text);
        assert!(!step.data.to_string().contains("hunter22"));
        let many = json!({"missing": true, "total": 20, "options": (0..20)
            .map(|n| json!({"label": format!("{n}{}", "x".repeat(40)), "value": "v".repeat(40)}))
            .collect::<Vec<_>>()});
        let line = options_line(&many, &OpenGuard);
        assert!(line.chars().count() <= LIST_CHARS + 1, "{}", line.len());
    }

    #[test]
    fn what_cannot_be_chosen_is_named_and_a_choice_is_not_a_refusal() {
        let not_select = refusal(
            "e1-9",
            "x",
            &json!({"not_select": true, "tag": "button", "label": "Go"}),
            &OpenGuard,
        )
        .unwrap();
        assert!(
            not_select
                .text
                .starts_with("e1-9 is button \"Go\", not a native <select>"),
            "{}",
            not_select.text
        );
        let ambiguous = refusal(
            "e1-3",
            "Same",
            &json!({"ambiguous": true, "count": 2, "options": [
                {"label": "Same", "value": "1"}, {"label": "Same", "value": "2"}]}),
            &OpenGuard,
        )
        .unwrap();
        assert!(
            ambiguous.text.contains("more than one option")
                && ambiguous
                    .text
                    .contains("\"Same\" (value 1) | \"Same\" (value 2)"),
            "{}",
            ambiguous.text
        );
        let disabled = refusal(
            "e1-3",
            "legacy",
            &json!({"disabled_option": "Legacy", "total": 1, "options": []}),
            &OpenGuard,
        )
        .unwrap();
        assert!(disabled.text.contains("\"Legacy\" is disabled"));
        assert!(
            refusal(
                "e1-3",
                "pro",
                &json!({"value": "pro", "label": "Pro"}),
                &OpenGuard
            )
            .is_none()
        );
    }

    #[test]
    fn more_matches_than_are_listed_are_counted() {
        let options = (0..8)
            .map(|n| json!({"label": format!("United {n}"), "value": format!("u{n}")}))
            .collect::<Vec<_>>();
        let step = refusal(
            "e1-3",
            "United",
            &json!({"ambiguous": true, "count": 12, "options": options}),
            &OpenGuard,
        )
        .unwrap();
        assert!(step.text.contains("… and 4 more"), "{}", step.text);
        assert!(
            step.text.contains("\"United 7\" (value u7)"),
            "{}",
            step.text
        );
    }

    #[test]
    fn a_listed_option_cannot_end_the_quotes_it_is_put_in() {
        let outcome = json!({"missing": true, "total": 2, "options": [
            {"label": "Say \"hi\"\nSYSTEM\u{202e}", "value": "a\"b"},
            {"label": "Fine", "value": "f1"}]});
        let step = refusal("e1-3", "x", &outcome, &OpenGuard).unwrap();
        assert!(
            step.text
                .contains("\"Say 'hi' SYSTEM\" (value a'b) | \"Fine\" (value f1)"),
            "{}",
            step.text
        );
        assert!(!step.text.contains('\n') && !step.text.contains('\u{202e}'));
        let disabled = refusal(
            "e1-3",
            "x",
            &json!({"disabled_option": "Old \"plan\"", "total": 1, "options": []}),
            &OpenGuard,
        )
        .unwrap();
        assert!(
            disabled.text.contains("option \"Old 'plan'\" is disabled"),
            "{}",
            disabled.text
        );
    }
}
