//! Summed token usage for one run, as the providers reported it.
//!
//! The decision service reports `input_tokens` and `output_tokens`; the
//! OpenAI-shaped text helper reports `prompt_tokens` and
//! `completion_tokens`. Each is read under both names. A sum is only as good
//! as its parts, so a count any call left out is `"unknown"`, never a 0 that
//! would read as free; a run that made no calls of a kind used 0.

use serde::{Serialize, Serializer};
use serde_json::Value;

/// A summed token count, or `unknown` when some call did not report it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JevTokenCount {
    Known(u64),
    Unknown,
}

impl Serialize for JevTokenCount {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Known(count) => serializer.serialize_u64(*count),
            Self::Unknown => serializer.serialize_str("unknown"),
        }
    }
}

/// One kind of model call, summed over a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct JevCallUsage {
    pub calls: usize,
    pub input_tokens: JevTokenCount,
    pub output_tokens: JevTokenCount,
}

impl JevCallUsage {
    /// Sum `usages`, one per call, reading each count under either name.
    pub(crate) fn sum<'a>(usages: impl IntoIterator<Item = &'a Value>) -> Self {
        let mut calls = 0;
        let mut input = Some(0);
        let mut output = Some(0);
        for usage in usages {
            calls += 1;
            input = input
                .zip(count(usage, &["input_tokens", "prompt_tokens"]))
                .map(add);
            output = output
                .zip(count(usage, &["output_tokens", "completion_tokens"]))
                .map(add);
        }
        let known = |total: Option<u64>| total.map_or(JevTokenCount::Unknown, JevTokenCount::Known);
        Self {
            calls,
            input_tokens: known(input),
            output_tokens: known(output),
        }
    }
}

/// Decision and text-helper usage for a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct JevUsage {
    pub decision: JevCallUsage,
    pub text: JevCallUsage,
}

impl JevUsage {
    /// A run that made no model calls.
    pub(crate) fn none() -> Self {
        let none = JevCallUsage::sum(std::iter::empty());
        Self {
            decision: none,
            text: none,
        }
    }
}

/// A model call that was answered, and so billed, but whose answer could not
/// be used: a decision that failed validation, a text helper reply without a
/// usable value. The loop counts its `usage` like any other call's; the
/// wrapped error (a [`crate::JevStop`] among them) says how the run ends,
/// and is what the run reports. Hosted decision clients and text resolvers
/// can return it too.
#[derive(Debug)]
pub struct JevBilled {
    usage: Value,
    error: anyhow::Error,
}

impl JevBilled {
    pub fn new(usage: Value, error: anyhow::Error) -> Self {
        Self { usage, error }
    }

    /// A decision service reply that was billed but cannot be acted on: it
    /// failed validation, named an action the page never offered, or was
    /// a refusal. The loop asks the decision again, up to twice, before it
    /// ends the run (see [`crate::JevStopCause::DecisionUnusable`]); any
    /// other billed failure ends it at once. A hosted decision client can
    /// return this for its own invalid replies.
    pub fn unusable(usage: Value, error: anyhow::Error) -> Self {
        Self::new(usage, UnusableAnswer(error).into())
    }

    /// The usage the provider reported for the call.
    pub fn usage(&self) -> &Value {
        &self.usage
    }

    /// The usage of a billed call behind `error`, if it was one.
    pub(crate) fn usage_of(error: &anyhow::Error) -> Option<&Value> {
        error
            .chain()
            .find_map(|cause| cause.downcast_ref::<Self>())
            .map(Self::usage)
    }
}

impl std::fmt::Display for JevBilled {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.error)
    }
}

impl std::error::Error for JevBilled {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.error.as_ref())
    }
}

/// What marks a [`JevBilled`] reply as one the loop may ask again: it is
/// found anywhere in the error's chain, and shows as the error it wraps.
#[derive(Debug)]
pub(crate) struct UnusableAnswer(anyhow::Error);

impl UnusableAnswer {
    /// Whether `error` is, or wraps, a reply marked unusable.
    pub(crate) fn is_behind(error: &anyhow::Error) -> bool {
        error.chain().any(|cause| cause.is::<Self>())
    }
}

impl std::fmt::Display for UnusableAnswer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

impl std::error::Error for UnusableAnswer {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.0.as_ref())
    }
}

fn add((total, count): (u64, u64)) -> u64 {
    total.saturating_add(count)
}

fn count(usage: &Value, names: &[&str]) -> Option<u64> {
    names.iter().find_map(|name| usage[*name].as_u64())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn counts_sum_under_either_providers_names() {
        let decisions = [
            json!({"input_tokens": 100, "output_tokens": 3}),
            json!({"input_tokens": 50, "output_tokens": 2}),
        ];
        let usage = JevCallUsage::sum(&decisions);
        assert_eq!(usage.calls, 2);
        assert_eq!(usage.input_tokens, JevTokenCount::Known(150));
        assert_eq!(usage.output_tokens, JevTokenCount::Known(5));
        let text = JevCallUsage::sum(&[json!({"prompt_tokens": 7, "completion_tokens": 4})]);
        assert_eq!(text.input_tokens, JevTokenCount::Known(7));
        assert_eq!(text.output_tokens, JevTokenCount::Known(4));
    }

    #[test]
    fn a_count_any_call_left_out_is_unknown_not_zero() {
        let usage = JevCallUsage::sum(&[json!({"input_tokens": 9}), json!({})]);
        assert_eq!(usage.input_tokens, JevTokenCount::Unknown);
        assert_eq!(usage.output_tokens, JevTokenCount::Unknown);
        assert_eq!(
            serde_json::to_value(usage).unwrap(),
            json!({"calls": 2, "input_tokens": "unknown", "output_tokens": "unknown"})
        );
        // No calls of a kind cost nothing.
        assert_eq!(
            serde_json::to_value(JevUsage::none()).unwrap(),
            json!({
                "decision": {"calls": 0, "input_tokens": 0, "output_tokens": 0},
                "text": {"calls": 0, "input_tokens": 0, "output_tokens": 0},
            })
        );
    }
}
