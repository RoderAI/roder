//! What an action did on Roder Desktop's browser, in the sentence the paired
//! extension's results lead with.
//!
//! Before an input, the page is read once, briefly; the action's own result
//! carries the page after it. The two are compared by the same function the
//! extension path uses, so one page changing one way is described by one
//! sentence whichever browser showed it.

use crate::direct::{DirectSession, DirectStep};
use crate::observed::{Comparison, Observed, clip, compare};
use crate::session::UNTRUSTED_NOTE;

/// Whether `short` is an input to the page, whose effect on it is described: a
/// look or a navigation is a page, not the effect of an action.
fn is_input(short: &str) -> bool {
    matches!(short, "click" | "type" | "scroll" | "key" | "select")
}

/// The page before `short`, when `short` is an input that changes the page.
pub(crate) async fn before(session: &mut DirectSession, short: &str) -> Option<Observed> {
    if !is_input(short) {
        return None;
    }
    let look = session.brief_look().await?;
    Some(Observed::from_look(&look))
}

/// Put the outcome first in an input's result. A step that failed, or that
/// the owner's rules stopped, did not do anything to describe; and a look or a
/// navigation has no outcome, as on the extension.
pub(crate) fn lead(step: &mut DirectStep, short: &str, before: Option<Observed>) {
    if !is_input(short) || step.is_error || step.stop.is_some() || !step.data["page"].is_object() {
        return;
    }
    let after = Observed::from_look(&step.data["page"]);
    let mut comparison = compare(before.as_ref(), &after);
    if step.opened_tab.is_some() {
        // The page now shown is another tab's; it was not what the action changed.
        comparison = Comparison {
            sentence: format!(
                "A new tab opened and is now the one driven: {}.",
                clip(&after.url, 110)
            ),
            compared: true,
            changed: true,
            ..Comparison::default()
        };
    }
    let dialogs = step.data["dialogs"].as_array().map_or(0, Vec::len);
    if dialogs > 0 {
        comparison.also(match dialogs {
            1 => "a dialog was answered (listed below)",
            _ => "dialogs were answered (listed below)",
        });
    }
    // The sentence can carry the page's title and address, so the label that
    // says page words are untrusted comes before it, as on the extension path.
    step.text = format!(
        "{UNTRUSTED_NOTE}\nOutcome: {}\n{}",
        comparison.sentence, step.text
    );
    step.data["outcome"] = comparison.data();
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn step(text: &str) -> DirectStep {
        DirectStep {
            text: text.to_string(),
            data: json!({"page": {"url": "https://a.test/", "title": "A", "text": "Hi",
                "elements": []}}),
            ..DirectStep::default()
        }
    }

    #[test]
    fn a_look_and_a_navigation_have_no_outcome_line() {
        for short in ["look", "navigate", "screenshot"] {
            let mut page = step("the page");
            lead(&mut page, short, None);
            assert_eq!(page.text, "the page", "{short}");
            assert!(page.data["outcome"].is_null(), "{short}");
        }
    }

    #[test]
    fn an_input_leads_with_its_outcome_even_with_nothing_to_compare() {
        for short in ["click", "type", "scroll", "key", "select"] {
            let mut input = step("Clicked.");
            lead(&mut input, short, None);
            assert!(
                input.text.ends_with(
                    "Outcome: No earlier observation of this tab to compare with.\nClicked."
                ),
                "{short}: {}",
                input.text
            );
            assert_eq!(input.data["outcome"]["compared"], json!(false), "{short}");
        }
    }

    #[test]
    fn a_failed_input_has_nothing_to_describe() {
        let mut failed = step("It did not work.");
        failed.is_error = true;
        lead(&mut failed, "click", None);
        assert_eq!(failed.text, "It did not work.");
    }
}
