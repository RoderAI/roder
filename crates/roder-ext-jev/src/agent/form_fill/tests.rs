//! Which fields a form call lists, and when a kept value is typed.

use std::sync::Mutex;

use async_trait::async_trait;
use serde_json::{Value, json};

use super::*;
use crate::engine::JevFormValues;

fn field(id: &str, node: u64, label: &str, form: Option<u64>) -> Value {
    json!({"id": id, "node": node, "ref": format!("j100.5-{node}"), "kind": "fill",
           "role": "textbox", "label": label, "value": "",
           "form": form.map(|id| json!({"id": id}))})
}

fn page(actions: Vec<Value>) -> Value {
    json!({"title": "Sign up", "text": "", "actions": actions})
}

fn labels(fields: &[&Value]) -> Vec<String> {
    fields
        .iter()
        .map(|field| field["label"].as_str().unwrap().to_string())
        .collect()
}

/// The chosen field first, then the form's other empty text fields; never
/// another form's, a combobox, a field holding text or a click.
#[test]
fn a_form_call_lists_the_forms_empty_text_fields_chosen_first() {
    let mut filled = field("e3", 3, "Phone", Some(1));
    filled["value"] = json!("555");
    let mut combobox = field("e4", 4, "City", Some(1));
    combobox["role"] = json!("combobox");
    let mut click = field("e5", 5, "Open Name", Some(1));
    click["kind"] = json!("click");
    let observation = page(vec![
        field("e1", 1, "Name", Some(1)),
        field("e2", 2, "Email", Some(1)),
        filled,
        combobox,
        click,
        field("e6", 6, "Search", Some(9)),
        field("e7", 7, "Notes", None),
        field("e8", 8, "Message", Some(1)),
    ]);
    let actions = observation["actions"].as_array().unwrap();
    let fields = form_fields(&actions[1], &observation).unwrap();
    assert_eq!(labels(&fields), ["Email", "Name", "Message"]);
}

/// A lone empty field, a field outside a form and a form with a secret
/// field get no form call.
#[test]
fn no_form_call_for_a_lone_field_no_form_or_a_secret() {
    let lone = page(vec![
        field("e1", 1, "Name", Some(1)),
        field("e2", 2, "Search", Some(2)),
    ]);
    let actions = lone["actions"].as_array().unwrap();
    assert!(form_fields(&actions[0], &lone).is_none());
    let loose = page(vec![
        field("e1", 1, "Name", None),
        field("e2", 2, "Email", None),
    ]);
    let actions = loose["actions"].as_array().unwrap();
    assert!(form_fields(&actions[0], &loose).is_none());
    let mut password = field("e3", 3, "Password", Some(1));
    password["input_type"] = json!("password");
    let login = page(vec![
        field("e1", 1, "Username", Some(1)),
        field("e2", 2, "Email", Some(1)),
        password,
    ]);
    let actions = login["actions"].as_array().unwrap();
    assert!(form_fields(&actions[0], &login).is_none());
    // The same form id in another document is another form.
    let mut elsewhere = field("e2", 2, "Email", Some(1));
    elsewhere["ref"] = json!("j200.5-2");
    let split = page(vec![field("e1", 1, "Name", Some(1)), elsewhere]);
    let actions = split["actions"].as_array().unwrap();
    assert!(form_fields(&actions[0], &split).is_none());
}

/// Answers a form call with fixed values by label, counting calls.
struct Writer {
    values: Vec<(&'static str, &'static str)>,
    asked: Mutex<Vec<Value>>,
}

#[async_trait]
impl JevTextValueResolver for Writer {
    async fn resolve(&self, _: &Value) -> anyhow::Result<JevTextValue> {
        unreachable!("only the form call is asked")
    }

    async fn resolve_form(&self, form: &Value) -> anyhow::Result<Option<JevFormValues>> {
        self.asked.lock().unwrap().push(form.clone());
        let values = form["fields"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|field| {
                let label = field["label"].as_str()?;
                let (_, value) = self.values.iter().find(|(known, _)| *known == label)?;
                Some((field["id"].as_str()?.to_string(), value.to_string()))
            })
            .collect();
        Ok(Some(JevFormValues {
            values,
            model: "writer".into(),
            latency_ms: 7,
            usage: json!({"input_tokens": 50}),
        }))
    }
}

async fn write(
    fill: &mut FormFill,
    writer: &Writer,
    chosen: &Value,
    observation: &Value,
    calls: &mut Vec<Value>,
) -> Option<JevTextValue> {
    let request = FormRequest {
        goal: "Sign up as Ada",
        observation,
        history: &[],
    };
    fill.write(writer, chosen, request, calls).await.unwrap()
}

/// One call: the chosen value now, the others kept for their own steps,
/// each typed once while the field still reads the same and is empty.
#[tokio::test]
async fn the_chosen_value_is_typed_now_and_the_others_are_kept() {
    let observation = page(vec![
        field("e1", 1, "Name", Some(1)),
        field("e2", 2, "Email", Some(1)),
        field("e3", 3, "Phone", Some(1)),
        field("e4", 4, "Company", Some(1)),
    ]);
    let writer = Writer {
        values: vec![
            ("Name", "Ada"),
            ("Email", "ada@example.com"),
            ("Phone", "555"),
        ],
        asked: Mutex::new(Vec::new()),
    };
    let (mut fill, mut calls) = (FormFill::default(), Vec::new());
    let actions = observation["actions"].as_array().unwrap();
    let now = write(&mut fill, &writer, &actions[0], &observation, &mut calls).await;
    assert_eq!(now.unwrap().value, "Ada");
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0]["fields"],
        json!(["Name", "Email", "Phone", "Company"])
    );
    assert_eq!(
        calls[0]["values"],
        json!(["Ada", "ada@example.com", "555", null])
    );
    // The form is asked once.
    assert!(
        write(&mut fill, &writer, &actions[0], &observation, &mut calls)
            .await
            .is_none()
    );
    assert_eq!(writer.asked.lock().unwrap().len(), 1);

    let kept = fill.take(&actions[1]).unwrap();
    assert_eq!(
        (kept.value.as_str(), kept.latency_ms),
        ("ada@example.com", 0)
    );
    assert_eq!(kept.model, "writer");
    assert!(
        fill.take(&actions[1]).is_none(),
        "a kept value is typed once"
    );
    // A field the reply gave nothing for has nothing kept.
    assert!(fill.take(&actions[3]).is_none());
    // A field that now holds text, or reads differently, is asked again.
    let mut phone = actions[2].clone();
    phone["value"] = json!("0");
    assert!(fill.take(&phone).is_none());
    assert!(
        fill.take(&actions[2]).is_none(),
        "dropped once seen changed"
    );
}

#[tokio::test]
async fn a_relabelled_field_does_not_take_its_kept_value() {
    let observation = page(vec![
        field("e1", 1, "Name", Some(1)),
        field("e2", 2, "Email", Some(1)),
    ]);
    let writer = Writer {
        values: vec![("Name", "Ada"), ("Email", "ada@example.com")],
        asked: Mutex::new(Vec::new()),
    };
    let (mut fill, mut calls) = (FormFill::default(), Vec::new());
    let actions = observation["actions"].as_array().unwrap();
    write(&mut fill, &writer, &actions[0], &observation, &mut calls).await;
    let mut relabelled = actions[1].clone();
    relabelled["label"] = json!("Email (work)");
    assert!(fill.take(&relabelled).is_none());
}

/// A chosen field the reply gives nothing for goes to the single-field
/// path; the others are still kept.
#[tokio::test]
async fn a_chosen_field_without_a_value_is_asked_alone() {
    let observation = page(vec![
        field("e1", 1, "Name", Some(1)),
        field("e2", 2, "Email", Some(1)),
    ]);
    let writer = Writer {
        values: vec![("Email", "ada@example.com")],
        asked: Mutex::new(Vec::new()),
    };
    let (mut fill, mut calls) = (FormFill::default(), Vec::new());
    let actions = observation["actions"].as_array().unwrap();
    assert!(
        write(&mut fill, &writer, &actions[0], &observation, &mut calls)
            .await
            .is_none()
    );
    assert_eq!(fill.take(&actions[1]).unwrap().value, "ada@example.com");
}

/// Typing, picking an option, clicking into a field or ticking a box keeps the values written;
/// a button click, Enter or a key may submit the form or show what it
/// rejected, so the fields left are asked for alone after it.
#[tokio::test]
async fn a_click_that_may_submit_drops_the_kept_values() {
    let observation = page(vec![
        field("e1", 1, "Name", Some(1)),
        field("e2", 2, "Email", Some(1)),
        field("e3", 3, "Phone", Some(1)),
    ]);
    let writer = Writer {
        values: vec![
            ("Name", "Ada"),
            ("Email", "ada@example.com"),
            ("Phone", "555"),
        ],
        asked: Mutex::new(Vec::new()),
    };
    let (mut fill, mut calls) = (FormFill::default(), Vec::new());
    let actions = observation["actions"].as_array().unwrap();
    write(&mut fill, &writer, &actions[0], &observation, &mut calls).await;
    fill.acted(&actions[0], &observation);
    fill.acted(&json!({"kind": "select", "role": "combobox"}), &observation);
    fill.acted(&json!({"kind": "click", "role": "checkbox"}), &observation);
    // Clicking into a field before typing in it, as the decision model does.
    let focus = json!({"kind": "click", "role": "generic", "ref": "j100.5-2"});
    fill.acted(&focus, &observation);
    assert_eq!(fill.take(&actions[1]).unwrap().value, "ada@example.com");
    let send = json!({"kind": "click", "role": "button", "label": "Send", "ref": "j100.5-9"});
    fill.acted(&send, &observation);
    assert!(fill.take(&actions[2]).is_none());
    write(&mut fill, &writer, &actions[0], &observation, &mut calls).await;
    assert_eq!(writer.asked.lock().unwrap().len(), 1, "still asked once");
}
