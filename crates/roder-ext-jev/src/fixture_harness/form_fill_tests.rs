//! Several fields of one form from one text call, on real pages: the first
//! field of a form asks for all its empty text fields at once, and the
//! steps that choose the others type the kept values without another call
//! (see `agent::form_fill`).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Context;
use async_trait::async_trait;
use serde_json::{Value, json};

use super::scripted::{PlanDecider, pick};
use crate::engine::{JevFormValues, JevRunResult, JevStatus, JevTextValue, JevTextValueResolver};

/// Values by label; answers form calls for the labels in `form` and
/// single calls for any label, recording what each call was asked.
struct Writer {
    values: Vec<(&'static str, &'static str)>,
    /// The labels a form call may give values for.
    form: Vec<&'static str>,
    /// A form call fails instead.
    failing: bool,
    forms: Mutex<Vec<Vec<String>>>,
    singles: Mutex<Vec<String>>,
}

impl Writer {
    fn new(values: &[(&'static str, &'static str)], form: &[&'static str]) -> Arc<Self> {
        Arc::new(Self {
            values: values.to_vec(),
            form: form.to_vec(),
            failing: false,
            forms: Mutex::new(Vec::new()),
            singles: Mutex::new(Vec::new()),
        })
    }

    fn value(&self, label: &str) -> Option<&'static str> {
        self.values
            .iter()
            .find(|(known, _)| *known == label)
            .map(|(_, value)| *value)
    }

    fn forms(&self) -> Vec<Vec<String>> {
        self.forms.lock().unwrap().clone()
    }

    fn singles(&self) -> Vec<String> {
        self.singles.lock().unwrap().clone()
    }
}

#[async_trait]
impl JevTextValueResolver for Writer {
    async fn resolve(&self, field_context: &Value) -> anyhow::Result<JevTextValue> {
        let label = field_context["field"]["label"].as_str().unwrap_or_default();
        self.singles.lock().unwrap().push(label.to_string());
        Ok(JevTextValue {
            value: self.value(label).context("no value")?.to_string(),
            model: "single".into(),
            latency_ms: 0,
            usage: json!({}),
        })
    }

    async fn resolve_form(&self, form_context: &Value) -> anyhow::Result<Option<JevFormValues>> {
        let fields = form_context["fields"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let label = |field: &Value| field["label"].as_str().unwrap_or_default().to_string();
        self.forms
            .lock()
            .unwrap()
            .push(fields.iter().map(label).collect());
        if self.failing {
            anyhow::bail!("unusable form reply");
        }
        let values = fields
            .iter()
            .filter(|field| self.form.contains(&label(field).as_str()))
            .filter_map(|field| {
                Some((
                    field["id"].as_str()?.to_string(),
                    self.value(&label(field))?.to_string(),
                ))
            })
            .collect();
        Ok(Some(JevFormValues {
            values,
            model: "form".into(),
            latency_ms: 0,
            usage: json!({"input_tokens": 100}),
        }))
    }
}

async fn run(
    harness: &super::Harness,
    page: &str,
    plan: Vec<super::scripted::Pick>,
    writer: Arc<Writer>,
) -> JevRunResult {
    harness
        .run(
            page,
            Arc::new(PlanDecider::new(plan)),
            Some(writer),
            Duration::from_secs(30),
        )
        .await
}

fn typed_by(result: &JevRunResult) -> Vec<Option<String>> {
    result
        .actions
        .iter()
        .filter(|action| action.kind == "fill")
        .map(|action| action.text_helper.clone())
        .collect()
}

const CONTACT: [(&str, &str); 3] = [
    ("Name", "Ada Lovelace"),
    ("Email", "ada@example.com"),
    ("Message", "Hello & welcome"),
];

/// Three fields, one text call: the steps after the first type the values
/// it wrote, and the form posts them.
#[tokio::test]
async fn a_form_is_filled_from_one_text_call() {
    let harness = harness_or_skip!();
    let writer = Writer::new(&CONTACT, &["Name", "Email", "Message"]);
    let plan = vec![
        pick("fill", "Name"),
        pick("fill", "Email"),
        pick("fill", "Message"),
        pick("click", "Send message"),
    ];
    let result = run(&harness, "contact.html", plan, writer.clone()).await;
    assert_eq!(result.status, JevStatus::Done, "{result:#?}");
    assert_eq!(writer.forms(), [["Name", "Email", "Message"]]);
    assert!(writer.singles().is_empty(), "{:?}", writer.singles());
    assert_eq!(result.text_calls, 1);
    assert_eq!(typed_by(&result), vec![Some("form".to_string()); 3]);
    let posts = harness.site.wait_for_posts(1, Duration::from_secs(5)).await;
    let post = posts.iter().find(|post| post.path == "/submit/contact");
    let post = post.expect("the contact form posted");
    for (field, value) in [
        ("name", "Ada Lovelace"),
        ("email", "ada@example.com"),
        ("message", "Hello & welcome"),
    ] {
        assert_eq!(post.field(field).as_deref(), Some(value), "{field}");
    }
}

/// Only the form's own empty text fields are listed. A field the reply
/// gave nothing for, and one the page filled after its value was written,
/// each get a call of their own.
#[tokio::test]
async fn a_field_left_out_or_changed_since_is_asked_alone() {
    let harness = harness_or_skip!();
    let writer = Writer::new(
        &[
            ("Name", "Ada"),
            ("Email", "ada@example.com"),
            ("Confirm email", "ada@example.com"),
            ("Phone", "555 0100"),
        ],
        &["Name", "Email", "Confirm email"],
    );
    let plan = vec![
        pick("fill", "Name"),
        pick("fill", "Email"),
        pick("fill", "Confirm email"),
        pick("fill", "Phone"),
        pick("click", "Sign up"),
    ];
    let result = run(&harness, "form-fill.html", plan, writer.clone()).await;
    assert_eq!(result.status, JevStatus::Done, "{result:#?}");
    assert_eq!(
        writer.forms(),
        [["Name", "Email", "Confirm email", "Phone"]],
        "Search belongs to another form"
    );
    assert_eq!(writer.singles(), ["Confirm email", "Phone"]);
    assert_eq!(result.text_calls, 3);
    let posts = harness.site.wait_for_posts(1, Duration::from_secs(5)).await;
    let post = posts.iter().find(|post| post.path == "/submit/signup");
    let post = post.expect("the sign-up form posted");
    assert_eq!(post.field("confirm").as_deref(), Some("ada@example.com"));
    assert_eq!(post.field("phone").as_deref(), Some("555 0100"));
}

/// A form with a password is never sent to the text model as a form: each
/// field is asked for alone, so the supervisor alone answers the password.
#[tokio::test]
async fn a_form_with_a_secret_field_is_not_batched() {
    let harness = harness_or_skip!();
    let writer = Writer::new(&[("Username", "ada"), ("Password", "pw-1234")], &[]);
    let plan = vec![
        pick("fill", "Username"),
        pick("fill", "Password"),
        pick("click", "Sign in"),
    ];
    let result = run(&harness, "login.html", plan, writer.clone()).await;
    assert_eq!(result.status, JevStatus::Done, "{result:#?}");
    assert!(writer.forms().is_empty());
    assert_eq!(writer.singles(), ["Username", "Password"]);
}

/// A form call that fails is not asked again for that form: every field
/// gets a call of its own and the run goes on.
#[tokio::test]
async fn a_failed_form_call_falls_back_to_one_call_per_field() {
    let harness = harness_or_skip!();
    let writer = Arc::new(Writer {
        failing: true,
        ..Arc::into_inner(Writer::new(&CONTACT, &[])).unwrap()
    });
    let plan = vec![
        pick("fill", "Name"),
        pick("fill", "Email"),
        pick("fill", "Message"),
        pick("click", "Send message"),
    ];
    let result = run(&harness, "contact.html", plan, writer.clone()).await;
    assert_eq!(result.status, JevStatus::Done, "{result:#?}");
    assert_eq!(writer.forms().len(), 1);
    assert_eq!(writer.singles(), ["Name", "Email", "Message"]);
}
