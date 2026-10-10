//! Secret fields: passwords and one-time codes.
//!
//! `snapshot.js` offers a password field, a field masked like one, and one
//! marked for a password or a one-time code as a fill action whose
//! `input_type` is `password` or `one-time-code`, with `filled` saying
//! whether it holds anything and `value` always empty: what such a field
//! holds is never read. The value Jev types into one is recorded as
//! [`SECRET`] on the step, and the loop scrubs it from everything it later
//! reads off the page, in case the page shows it back (a "show password"
//! toggle, an echo in an error message, a GET form's address).

use serde_json::Value;

/// What a step, a text-helper record or a scrubbed page shows instead of a
/// typed secret. Its length is not given.
pub(crate) const SECRET: &str = "[secret]";

/// A typed secret shorter than this is not scrubbed from the page: taking
/// every "12" out of the page text would garble it for a code nobody could
/// recognise there anyway.
const SCRUB_MIN_CHARS: usize = 4;

/// Whether an observed action targets a secret field.
pub(crate) fn is_secret(action: &Value) -> bool {
    matches!(
        action["input_type"].as_str(),
        Some("password" | "one-time-code")
    )
}

/// The secrets typed in one run, kept only in memory to scrub them from
/// what the page shows. A session hands them to its next call, so a value
/// typed in one call is still scrubbed from the pages later calls read.
#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct Secrets(Vec<String>, bool);

/// Never the values: a result is debug-printed in logs and test failures.
impl std::fmt::Debug for Secrets {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Secrets({} kept)", self.0.len())
    }
}

impl Secrets {
    /// Any secret input, including short values not retained for text scrubbing.
    pub(crate) fn has_input(&self) -> bool {
        self.1
    }

    /// How many are remembered.
    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }

    /// Remember every secret `other` holds too.
    pub(crate) fn extend(&mut self, other: &Secrets) {
        self.1 |= other.1;
        for secret in &other.0 {
            self.remember(secret);
        }
    }

    pub(crate) fn remember(&mut self, value: &str) {
        self.1 = true;
        let value = value.trim();
        if value.chars().count() >= SCRUB_MIN_CHARS && !self.0.iter().any(|known| known == value) {
            self.0.push(value.to_string());
        }
    }

    /// `text` with every remembered secret replaced by [`SECRET`].
    pub(crate) fn scrub(&self, text: &str) -> String {
        self.0.iter().fold(text.to_string(), |text, secret| {
            text.replace(secret.as_str(), SECRET)
        })
    }

    fn scrub_in_place(&self, value: &mut Value) {
        if let Value::String(text) = value
            && self.0.iter().any(|secret| text.contains(secret.as_str()))
        {
            *text = self.scrub(text);
        }
    }

    fn scrub_evidence(&self, value: &mut Value) {
        match value {
            Value::Array(values) => values.iter_mut().for_each(|v| self.scrub_evidence(v)),
            Value::Object(values) => values.values_mut().for_each(|v| self.scrub_evidence(v)),
            _ => self.scrub_in_place(value),
        }
    }

    /// Scrub the parts of an observation that leave the loop: its address,
    /// title and text (and the field values the text holds), each action's
    /// label, value, current value, context
    /// and section, the dialogs' messages, and the frames' origins and
    /// text. The freshness marker, page key
    /// and guards are compared with the live page and never leave it, so
    /// they are kept; so is a select option's value, which choosing it needs.
    pub(crate) fn scrub_observation(&self, observation: &mut Value) {
        if self.0.is_empty() {
            return;
        }
        for key in ["url", "title", "text"] {
            if let Some(value) = observation.get_mut(key) {
                self.scrub_in_place(value);
            }
        }
        for key in ["disabled_controls", "typed_values"] {
            if let Some(evidence) = observation.get_mut(key) {
                self.scrub_evidence(evidence);
            }
        }
        for action in observation["actions"].as_array_mut().into_iter().flatten() {
            if let Some(form) = action.get_mut("form") {
                self.scrub_evidence(form);
            }
            let select = action["kind"] == "select";
            for key in ["label", "value", "current_value", "context", "section"] {
                if key == "value" && select {
                    continue;
                }
                if let Some(value) = action.get_mut(key) {
                    self.scrub_in_place(value);
                }
            }
        }
        for dialog in observation["dialogs"].as_array_mut().into_iter().flatten() {
            if let Some(message) = dialog.get_mut("message") {
                self.scrub_in_place(message);
            }
        }
        for frame in observation["frames"].as_array_mut().into_iter().flatten() {
            for key in ["origin", "text"] {
                if let Some(value) = frame.get_mut(key) {
                    self.scrub_in_place(value);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

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
}
