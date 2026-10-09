//! Caller-defined UI postconditions, independent of the decision model's DONE.
//!
//! Each predicate is matched on the comparable form of both sides (see
//! [`text::fold`]): case, runs of whitespace (a line break between page
//! nodes, a no-break space) and zero-width characters do not matter. The
//! page's text leaves out what form fields hold, so a value the run typed in
//! cannot satisfy `text_contains` or hide from `text_absent`.
mod text;

use crate::engine::{JevRunResult, JevStatus, JevStopCause};
use crate::fallback::FinalPage;
use anyhow::{Context, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use text::{fold, without_echoes};

/// The longest a predicate may be.
const MAX_BYTES: usize = 4096;

/// What a check says it covers.
const SCOPE: &str = "Only the caller-specified URL/text predicates, compared ignoring case, \
    spacing and zero-width characters, against the page text on screen (6000 characters at most; \
    what form fields hold is left out). Not a proof of the entire natural-language goal.";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Completion {
    #[serde(default)]
    url_contains: String,
    #[serde(default)]
    text_contains: String,
    #[serde(default)]
    text_absent: String,
}
impl Completion {
    pub(crate) fn parse(args: &Value) -> anyhow::Result<Option<Self>> {
        let Some(raw) = args.get("success_condition").filter(|v| {
            !v.is_null() && v.as_str() != Some("") && !v.as_array().is_some_and(Vec::is_empty)
        }) else {
            return Ok(None);
        };
        let condition: Self = serde_json::from_value(raw.clone()).context(
            "success_condition must contain only string url_contains/text_contains/text_absent fields",
        )?;
        if condition.supplied().is_empty() {
            return Ok(None);
        }
        for (name, value) in condition.predicates() {
            ensure!(
                value.len() <= MAX_BYTES,
                "success_condition strings must be at most 4096 bytes"
            );
            // Only spaces would compare equal to anything: an empty string
            // is how a predicate is skipped.
            ensure!(
                value.is_empty() || !fold(value).is_empty(),
                "success_condition {name} has no visible characters; leave it empty to skip it"
            );
        }
        Ok(Some(condition))
    }
    fn predicates(&self) -> [(&'static str, &str); 3] {
        [
            ("url_contains", &self.url_contains),
            ("text_contains", &self.text_contains),
            ("text_absent", &self.text_absent),
        ]
    }
    /// The predicates the caller gave, which are the nonempty ones.
    fn supplied(&self) -> Vec<&'static str> {
        self.predicates()
            .into_iter()
            .filter(|(_, value)| !value.is_empty())
            .map(|(name, _)| name)
            .collect()
    }
    /// The predicates `page` does not satisfy.
    fn unmet(&self, page: &FinalPage) -> Vec<&'static str> {
        let url = fold(&page.url);
        let text = fold(&without_echoes(&page.visible_text, &page.typed_values));
        let mut unmet = Vec::new();
        if !self.url_contains.is_empty() && !url.contains(&fold(&self.url_contains)) {
            unmet.push("url_contains");
        }
        if !self.text_contains.is_empty() && !text.contains(&fold(&self.text_contains)) {
            unmet.push("text_contains");
        }
        if !self.text_absent.is_empty() && text.contains(&fold(&self.text_absent)) {
            unmet.push("text_absent");
        }
        unmet
    }
    /// What is not met: what `page` fails, or every predicate when no fresh
    /// observation could be made, since none can then be shown to hold.
    fn not_met(&self, page: Option<&FinalPage>) -> Vec<&'static str> {
        page.map_or_else(|| self.supplied(), |page| self.unmet(page))
    }
    pub(crate) fn matches(&self, page: &FinalPage) -> bool {
        self.unmet(page).is_empty()
    }
    pub(crate) fn report(&self, page: Option<&FinalPage>) -> Value {
        let unmet = self.not_met(page);
        json!({"status": if unmet.is_empty() {"passed"} else {"failed"}, "unmet": unmet,
          "source":"fresh browser observation", "url_contains":self.url_contains,
          "text_contains":self.text_contains, "text_absent":self.text_absent,
          "observation_available":page.is_some(), "scope":SCOPE})
    }
}
pub(crate) fn apply(result: &mut JevRunResult, condition: &Completion, page: Option<&FinalPage>) {
    if let Some(page) = page {
        result.url = page.url.clone();
        result.title = page.title.clone();
        result.visible_text = page.visible_text.clone();
        result.controls = serde_json::from_value(page.controls.clone()).unwrap_or_default();
        result.page = serde_json::from_value(page.page.clone()).ok();
        result.observed_elements = page.observed_elements;
        result.omitted = page.omitted;
    }
    let unmet = condition.not_met(page);
    if !unmet.is_empty() {
        result.status = JevStatus::Blocked;
        result.stop_cause = Some(JevStopCause::OutcomeMismatch);
        result.stopped_because = Some(format!(
            "Model reported DONE, but a fresh browser observation did not satisfy the caller's success_condition (not met: {})",
            unmet.join(", ")
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn condition(value: Value) -> Completion {
        Completion::parse(&json!({ "success_condition": value }))
            .unwrap()
            .unwrap()
    }

    fn page(url: &str, text: &str, typed: &[&str]) -> FinalPage {
        FinalPage {
            url: url.into(),
            visible_text: text.into(),
            typed_values: typed.iter().map(|value| value.to_string()).collect(),
            ..FinalPage::default()
        }
    }

    #[test]
    fn the_page_a_check_read_gives_the_count_of_what_was_not_offered() {
        use crate::engine::JevOmitted;
        let mut result = JevRunResult::before_start(
            JevStatus::Done,
            "https://a.test/",
            std::time::Duration::ZERO,
            "x",
        );
        result.omitted = JevOmitted {
            controls: 9,
            options: 0,
        };
        let condition = condition(json!({"text_contains": "ok"}));
        // No fresh read: the run's own count stays.
        apply(&mut result, &condition, None);
        assert_eq!(result.omitted.controls, 9);

        let mut fresh = page("https://a.test/", "ok", &[]);
        fresh.omitted = JevOmitted {
            controls: 0,
            options: 45,
        };
        apply(&mut result, &condition, Some(&fresh));
        assert_eq!(result.omitted, fresh.omitted);
    }

    #[test]
    fn optional_completion_accepts_all_documented_empty_values() {
        for value in [Value::Null, json!(""), json!([]), json!({})] {
            assert!(
                Completion::parse(&json!({"success_condition":value}))
                    .unwrap()
                    .is_none()
            );
        }
        assert!(Completion::parse(&json!({"success_condition":42})).is_err());
        assert!(
            Completion::parse(&json!({"success_condition":{"text_contains":"Submitted"}}))
                .unwrap()
                .is_some()
        );
        // Empty strings skip their predicate, all of them skip the check.
        assert!(
            Completion::parse(&json!({"success_condition":
                {"url_contains":"","text_contains":"","text_absent":""}}))
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn text_absent_is_a_predicate_with_the_same_limits() {
        let only = condition(json!({"text_absent":"Saving"}));
        assert_eq!(only.supplied(), ["text_absent"]);
        assert!(
            Completion::parse(&json!({"success_condition":{"text_absent":"x".repeat(4096)}}))
                .is_ok()
        );
        for name in ["url_contains", "text_contains", "text_absent"] {
            let too_long = Completion::parse(&json!({"success_condition":{name:"x".repeat(4097)}}));
            assert!(too_long.is_err(), "{name}");
        }
        assert!(Completion::parse(&json!({"success_condition":{"text_absent":7}})).is_err());
    }

    #[test]
    fn a_predicate_of_nothing_visible_is_refused_not_skipped() {
        for name in ["url_contains", "text_contains", "text_absent"] {
            for blank in [" ", "\n\t", "\u{a0}", "\u{200b}", " \u{feff} "] {
                let error = Completion::parse(&json!({"success_condition":{name:blank}}))
                    .unwrap_err()
                    .to_string();
                assert!(
                    error.contains(name) && error.contains("no visible"),
                    "{error}"
                );
            }
        }
        // Next to a real predicate it is still refused: the caller meant something.
        assert!(
            Completion::parse(&json!({"success_condition":
                {"text_contains":"Done","text_absent":" "}}))
            .is_err()
        );
    }

    #[test]
    fn text_contains_ignores_case_line_breaks_no_break_spaces_and_zero_width() {
        let wanted = condition(json!({"text_contains":"count: 1"}));
        assert!(wanted.matches(&page("", "Count:\n1", &[])));
        assert!(wanted.matches(&page("", "Heading\nCOUNT:\u{a0}\u{a0}1\nOther", &[])));
        assert!(wanted.matches(&page("", "Co\u{200b}unt:\n\t1", &[])));
        assert!(!wanted.matches(&page("", "Count:\n2", &[])));
        // Both sides fold: the caller's own line breaks and capitals too.
        let spaced = condition(json!({"text_contains":" ORDER\n\u{a0}confirmed\u{200b} "}));
        assert!(spaced.matches(&page("", "Your order confirmed.", &[])));
        // The old exact form still matches.
        assert!(
            condition(json!({"text_contains":"Count:\n1"})).matches(&page("", "Count:\n1", &[]))
        );
    }

    #[test]
    fn url_contains_ignores_case() {
        let wanted = condition(json!({"url_contains":"/Search?Q=Boots"}));
        assert!(wanted.matches(&page("https://shop.test/search?q=boots&p=2", "", &[])));
        assert!(!wanted.matches(&page("https://shop.test/search?q=shoes", "", &[])));
    }

    #[test]
    fn text_absent_fails_while_the_text_shows_in_any_spelling() {
        let wanted = condition(json!({"text_absent":"saving order"}));
        assert!(!wanted.matches(&page("", "Saving\u{a0}ORDER…", &[])));
        assert!(!wanted.matches(&page("", "Sa\u{200b}ving\norder", &[])));
        assert!(wanted.matches(&page("", "Order saved", &[])));
        assert!(wanted.matches(&page("", "", &[])));
    }

    #[test]
    fn every_given_predicate_must_hold_and_the_report_names_the_unmet() {
        let wanted = condition(json!({"url_contains":"/done","text_contains":"thanks",
            "text_absent":"error"}));
        let all = page("https://x.test/done", "Thanks!", &[]);
        assert!(wanted.matches(&all));
        assert_eq!(wanted.report(Some(&all))["status"], "passed");
        assert_eq!(wanted.report(Some(&all))["unmet"], json!([]));
        let none = page("https://x.test/form", "Error: try again", &[]);
        let report = wanted.report(Some(&none));
        assert_eq!(report["status"], "failed");
        assert_eq!(
            report["unmet"],
            json!(["url_contains", "text_contains", "text_absent"])
        );
        assert_eq!(report["observation_available"], true);
        let one = page("https://x.test/done", "Thanks! Error", &[]);
        assert_eq!(wanted.report(Some(&one))["unmet"], json!(["text_absent"]));
    }

    #[test]
    fn without_an_observation_nothing_is_shown_to_hold() {
        // Not even an absence: nothing was seen.
        let wanted = condition(json!({"text_absent":"error","url_contains":"/done"}));
        let report = wanted.report(None);
        assert_eq!(report["status"], "failed");
        assert_eq!(report["observation_available"], false);
        assert_eq!(report["unmet"], json!(["url_contains", "text_absent"]));
        assert!(report["scope"].as_str().unwrap().contains("Not a proof"));
        assert!(!report.to_string().to_ascii_lowercase().contains("verified"));
    }

    #[test]
    fn what_a_field_holds_is_not_page_text() {
        let wanted = condition(json!({"text_contains":"order confirmed"}));
        let typed = page("", "Note\norder confirmed\nFinish", &["order confirmed"]);
        assert!(!wanted.matches(&typed));
        let shown_too = page(
            "",
            "Note\norder confirmed\nOrder\u{a0}Confirmed",
            &["order confirmed"],
        );
        assert!(wanted.matches(&shown_too));
        // An absence needs the page, not the field, to be free of it.
        assert!(condition(json!({"text_absent":"order confirmed"})).matches(&typed));
        assert!(!condition(json!({"text_absent":"order confirmed"})).matches(&shown_too));
    }

    #[test]
    fn a_failed_done_says_which_predicates_were_not_met() {
        use crate::engine::JevRunResult;
        let mut result = JevRunResult::before_start(
            JevStatus::Done,
            "https://x.test/",
            std::time::Duration::ZERO,
            String::new(),
        );
        let wanted = condition(json!({"text_contains":"thanks","text_absent":"error"}));
        apply(
            &mut result,
            &wanted,
            Some(&page("https://x.test/", "Error", &[])),
        );
        assert_eq!(result.status, JevStatus::Blocked);
        assert_eq!(result.stop_cause, Some(JevStopCause::OutcomeMismatch));
        let why = result.stopped_because.unwrap();
        assert!(why.contains("not met: text_contains, text_absent"), "{why}");
    }
}
