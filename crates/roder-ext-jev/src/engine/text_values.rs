//! Where the text a fill types comes from: a resolver, asked one field at a
//! time, or, when it can, for several empty fields of one form at once.

use std::collections::BTreeMap;

use async_trait::async_trait;
use serde_json::Value;

/// A value generated for a fill action.
#[derive(Debug, Clone)]
pub struct JevTextValue {
    pub value: String,
    pub model: String,
    pub latency_ms: u64,
    pub usage: Value,
}

/// The values one call wrote for several fields of one form.
#[derive(Debug, Clone)]
pub struct JevFormValues {
    /// Each field given a value, by the action id the form context listed
    /// it under. A field the goal gives no value for is left out.
    pub values: BTreeMap<String, String>,
    pub model: String,
    pub latency_ms: u64,
    pub usage: Value,
}

/// Resolves a fill value from JEV's field context.
///
/// The preferred source for a password or one-time code (the field
/// context's `field.input_type` is `password` or `one-time-code`):
/// supervisors may return an opaque value reference and resolve it only
/// while executing the browser action, so the secret never enters the goal,
/// the decision request, the text model or the JEV trace. Whatever is
/// returned for a secret field is recorded as `[secret]`. A resolver with
/// no value should fail with [`JevStop`](super::JevStop) and
/// [`JevStatus::NeedsInput`](super::JevStatus::NeedsInput),
/// naming the field; Roder's own text helper does, and never types a
/// password or code that is not written in the goal.
#[async_trait]
pub trait JevTextValueResolver: Send + Sync {
    async fn resolve(&self, field_context: &Value) -> anyhow::Result<JevTextValue>;

    /// The values of several empty fields of one form from one call (after
    /// fastbrowse's form writer). The form context is the field context of
    /// the field being filled, with `field` replaced by `fields`: that
    /// field first, then the form's other empty text fields, each with its
    /// `id`. A form is only offered when none of its fields is a password or
    /// one-time code, so no secret field is ever listed. Jev types the
    /// first field's value now and keeps the others for the steps that
    /// choose those fields while they stay as they were; a field left out
    /// is asked for alone with [`Self::resolve`].
    ///
    /// `Ok(None)` (the default) means the resolver answers one field at a
    /// time; an error falls back to that too, except one that ends the run
    /// as
    /// [`JevStatus::Unavailable`](super::JevStatus::Unavailable).
    async fn resolve_form(&self, form_context: &Value) -> anyhow::Result<Option<JevFormValues>> {
        let _ = form_context;
        Ok(None)
    }
}
