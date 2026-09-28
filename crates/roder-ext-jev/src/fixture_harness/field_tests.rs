//! How fields are described on a real DOM (`fields.html`): names from the
//! text around an unlabelled field, never from a field's own content.

use serde_json::Value;

use super::scripted::find;

fn label_of(observation: &Value, id: &str) -> String {
    let node = |action: &Value| action["node"].as_i64();
    let target = observation["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|action| action["dom_id"] == id)
        .and_then(node);
    observation["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|action| node(action) == target && action["kind"] != "click")
        .map(|action| {
            action["label"]
                .as_str()
                .unwrap_or_default()
                .split(" → ")
                .next()
                .unwrap_or_default()
                .to_string()
        })
        .unwrap_or_default()
}

/// Tag each action with its element's DOM id, for this test only.
async fn observe_with_ids(page: &mut crate::page::Page) -> Value {
    let mut observation = page.observe().await.unwrap();
    let ids = page
        .evaluate(
            "Object.fromEntries([...window.__jevFast.nodes].map(([n, e]) => [n, e.id || null]))",
        )
        .await
        .unwrap();
    for action in observation["actions"].as_array_mut().unwrap() {
        if let Some(node) = action["node"].as_i64() {
            action["dom_id"] = ids[node.to_string()].clone();
        }
    }
    observation
}

#[tokio::test]
async fn an_unlabelled_field_is_named_by_the_text_around_it() {
    let harness = harness_or_skip!();
    let mut page = harness.open("fields.html").await.unwrap();
    let observation = observe_with_ids(&mut page).await;
    let named = |id| label_of(&observation, id);

    // A label with no "for", a header cell in the row, text before and text
    // after the field.
    assert_eq!(named("genre"), "Genre");
    assert_eq!(named("director"), "Director");
    assert_eq!(named("year"), "Year:");
    assert_eq!(named("email"), "Contact email");
    // A placeholder comes first; two fields in one box name neither.
    assert_eq!(named("query"), "Title or actor");
    assert_eq!(named("low"), "textbox");
    assert_eq!(named("high"), "textbox");
    // A textarea is not named by its text, nor a select by its options.
    assert_eq!(named("note"), "Note");
    assert_eq!(named("rating"), "combobox");
    // A form's header is not the label of the field below it.
    assert_eq!(named("reply"), "textbox");
    assert_eq!(named("guess"), "spinbutton");
    // Each text field says what kind it is.
    let kind = |label| find(&observation, "fill", label).unwrap()["input_type"].clone();
    assert_eq!(kind("Note"), "textarea");
    assert_eq!(kind("Genre"), "text");
    assert_eq!(kind("Contact email"), "email");
    assert_eq!(kind("Title or actor"), "search");
}

#[tokio::test]
async fn what_a_text_field_holds_is_page_text_but_never_a_password() {
    let harness = harness_or_skip!();
    let mut page = harness.open("fields.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let text = observation["text"].as_str().unwrap();
    assert!(text.contains("Meet at the north gate at nine."), "{text}");
    assert!(!text.contains("hunter2"), "{text}");
    // The textarea's value, not its initial text, once it changes.
    let (_, next) = super::act_on(&mut page, &observation, "fill", "Genre", Some("Film noir"))
        .await
        .unwrap();
    page.evaluate("note.value = 'Changed plans'").await.unwrap();
    let next = page.observe().await.unwrap_or(next);
    let text = next["text"].as_str().unwrap();
    assert!(text.contains("Film noir"), "{text}");
    assert!(text.contains("Changed plans"), "{text}");
    assert!(!text.contains("north gate"), "{text}");
}
