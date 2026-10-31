//! Secret fields: passwords and one-time codes.
//!
//! `snapshot.js` offers a password field, a field masked like one, and one
//! marked for a password or a one-time code as a fill action whose
//! `input_type` is `password` or `one-time-code`, with `filled` saying
//! whether it holds anything and `value` always empty: what such a field
//! holds is never read. The value Jev types into one is recorded as
//! [`SECRET`] on the step, and the loop scrubs it from everything it later
//! reads off the page, in case the page shows it back (a "show password"
//! toggle, an echo in an error message, a GET form's address). It is found
//! in every form a page or an address carries it in (see [`forms`]): a GET
//! form puts `p@ss word` in the address as `p%40ss+word`.

mod forms;
#[cfg(test)]
mod tests;

use serde_json::Value;

use forms::{forms, hosts, origin_of};

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

/// One typed secret: every form it is carried in, and the origins it was
/// typed on.
#[derive(Clone, PartialEq, Eq)]
struct Secret {
    forms: Vec<String>,
    typed_on: Vec<String>,
}

/// The secrets typed in one run, kept only in memory to scrub them from
/// what the page shows. A session hands them to its next call, so a value
/// typed in one call is still scrubbed from the pages later calls read.
#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct Secrets {
    kept: Vec<Secret>,
    /// Any secret was typed, including one too short to keep.
    typed: bool,
}

/// Never the values: a result is debug-printed in logs and test failures.
impl std::fmt::Debug for Secrets {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Secrets({} kept)", self.kept.len())
    }
}

impl Secrets {
    /// Any secret input, including short values not retained for text scrubbing.
    pub(crate) fn has_input(&self) -> bool {
        self.typed
    }

    /// How many are remembered.
    pub(crate) fn len(&self) -> usize {
        self.kept.len()
    }

    /// Remember every secret `other` holds too.
    pub(crate) fn extend(&mut self, other: &Secrets) {
        self.typed |= other.typed;
        for secret in &other.kept {
            match self
                .kept
                .iter_mut()
                .find(|known| known.forms == secret.forms)
            {
                Some(known) => {
                    for origin in &secret.typed_on {
                        if !known.typed_on.contains(origin) {
                            known.typed_on.push(origin.clone());
                        }
                    }
                }
                None => self.kept.push(secret.clone()),
            }
        }
    }

    /// Remember a secret typed on a page whose address is not known.
    pub(crate) fn remember(&mut self, value: &str) {
        self.remember_at(value, None);
    }

    /// Remember a secret typed on the page at `url`. The host of an address
    /// on that origin keeps the value: a password that is an ordinary word
    /// must not blank out the site's own name in every address the run
    /// reports, and the site publishes its host anyway.
    pub(crate) fn remember_at(&mut self, value: &str, url: Option<&str>) {
        self.typed = true;
        let value = value.trim();
        if value.chars().count() < SCRUB_MIN_CHARS {
            return;
        }
        let secret = Secret {
            forms: forms(value),
            typed_on: url.and_then(origin_of).into_iter().collect(),
        };
        self.extend(&Secrets {
            kept: vec![secret],
            typed: true,
        });
    }

    /// `text` with every remembered secret replaced by [`SECRET`], in any
    /// form it is carried in: as typed, with its whitespace collapsed as
    /// accessible names do, percent-encoded (`%20` or `+` for a space, as a
    /// query string or `encodeURIComponent` writes it) or escaped inside a
    /// JSON string. A value already replaced is left, so scrubbing twice
    /// changes nothing.
    pub(crate) fn scrub(&self, text: &str) -> String {
        let spans = self.spans(text);
        if spans.is_empty() {
            return text.to_string();
        }
        let mut out = String::with_capacity(text.len());
        let mut at = 0;
        for (start, end) in spans {
            out.push_str(&text[at..start]);
            out.push_str(SECRET);
            at = end;
        }
        out.push_str(&text[at..]);
        out
    }

    /// Where the secrets sit in `text`, as byte ranges merged where they
    /// overlap: never inside a [`SECRET`] marker, nor wholly inside the host
    /// of an address on an origin the secret was typed on.
    fn spans(&self, text: &str) -> Vec<(usize, usize)> {
        if self.kept.is_empty() {
            return Vec::new();
        }
        let markers: Vec<(usize, usize)> = text
            .match_indices(SECRET)
            .map(|(at, _)| (at, at + SECRET.len()))
            .collect();
        let hosts = match self.kept.iter().any(|secret| !secret.typed_on.is_empty()) {
            true => hosts(text),
            false => Vec::new(),
        };
        let mut found = Vec::new();
        for secret in &self.kept {
            for form in &secret.forms {
                let mut from = 0;
                while let Some(offset) = text[from..].find(form.as_str()) {
                    let start = from + offset;
                    let end = start + form.len();
                    let within = |lo: usize, hi: usize| lo <= start && end <= hi;
                    let published = hosts.iter().any(|(lo, hi, origin)| {
                        within(*lo, *hi) && secret.typed_on.contains(origin)
                    });
                    if !published && !markers.iter().any(|(lo, hi)| within(*lo, *hi)) {
                        found.push((start, end));
                    }
                    // One character on, so overlapping occurrences are found.
                    from = start + text[start..].chars().next().map_or(1, char::len_utf8);
                }
            }
        }
        found.sort_by_key(|(start, end)| (*start, std::cmp::Reverse(*end)));
        let mut merged: Vec<(usize, usize)> = Vec::new();
        for (start, end) in found {
            match merged.last_mut() {
                Some(last) if start < last.1 => last.1 = last.1.max(end),
                _ => merged.push((start, end)),
            }
        }
        merged
    }

    fn scrub_in_place(&self, value: &mut Value) {
        if let Value::String(text) = value {
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
    /// title and text (and the field values the text holds), its DOM and
    /// semantic snapshots, each action's label, value, current value,
    /// context, section and form, the dialogs' messages, and the frames'
    /// origins and text. The freshness marker, page key and guards are
    /// compared with the live page and never leave it, so they are kept; so
    /// is a select option's value, which choosing it needs.
    pub(crate) fn scrub_observation(&self, observation: &mut Value) {
        if self.kept.is_empty() {
            return;
        }
        for key in ["url", "title", "text", "dom_snapshot"] {
            if let Some(value) = observation.get_mut(key) {
                self.scrub_in_place(value);
            }
        }
        if let Some(snapshot) = observation.get_mut("snapshot") {
            self.scrub_evidence(snapshot);
        }
        crate::semantic::refresh(observation);
        for key in ["disabled_controls", "typed_values"] {
            if let Some(evidence) = observation.get_mut(key) {
                self.scrub_evidence(evidence);
            }
        }
        for action in observation["actions"].as_array_mut().into_iter().flatten() {
            for evidence in ["form", "submits"] {
                if let Some(evidence) = action.get_mut(evidence) {
                    self.scrub_evidence(evidence);
                }
            }
            let select = action["kind"] == "select";
            for key in [
                "label",
                "value",
                "current_value",
                "context",
                "section",
                "href",
            ] {
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
