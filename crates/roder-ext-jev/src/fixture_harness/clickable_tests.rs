//! Generic clickables on a real DOM (`clickables.html`): elements with a
//! click listener, an onclick handler, a tabindex or a pointer cursor are
//! offered; wrappers, delegation roots and anything inside a control are not.

use serde_json::{Value, json};

use super::act_on;
use super::scripted::find;

fn offered(observation: &Value) -> Vec<(String, String)> {
    observation["actions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|action| action["kind"] == "click")
        .map(|action| {
            (
                action["role"].as_str().unwrap_or_default().to_string(),
                action["label"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect()
}

#[tokio::test]
async fn listeners_handlers_tabindex_and_pointer_regions_are_offered() {
    let harness = harness_or_skip!();
    let mut page = harness.open("clickables.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let offered = offered(&observation);
    let has = |role: &str, label: &str| offered.iter().any(|(r, l)| r == role && l == label);

    // A span with a click listener, an onclick handler, a tabindex, an
    // anchor with no href but a listener, and a treeitem.
    for label in ["In", "Out", "Inline handler", "Focusable tile", "Prev"] {
        assert!(has("button", label), "{label}: {offered:?}");
    }
    assert!(has("treeitem", "Documents"), "{offered:?}");
    // Unnamed icons, by their picture's file; a hashed file name names nothing.
    assert!(has("button", "delete"), "{offered:?}");
    assert!(has("button", "icon star"), "{offered:?}");
    assert!(has("button", "button"), "{offered:?}");
    // A floating list's item, once, as an option.
    assert_eq!(
        offered
            .iter()
            .filter(|(_, l)| l == "Paris (CDG)")
            .collect::<Vec<_>>(),
        [&("option".to_string(), "Paris (CDG)".to_string())],
        "{offered:?}"
    );
    // The mail rows (a pointer region with a delegated handler) and the stars
    // inside them (their own listeners) are both offered.
    assert!(
        offered.iter().any(|(_, label)| label.starts_with("Ada")),
        "{offered:?}"
    );
    assert_eq!(
        offered.iter().filter(|(_, label)| label == "Star").count(),
        2,
        "{offered:?}"
    );
    // Not a named anchor, not a list item that only wraps its link, not a
    // toolbar that listens for its buttons, not a button's own icon.
    assert!(!offered.iter().any(|(_, l)| l.starts_with("Named anchor")));
    assert_eq!(
        offered.iter().filter(|(_, l)| l == "Home").count(),
        1,
        "{offered:?}"
    );
    assert!(
        !offered.iter().any(|(_, l)| l.starts_with("Tools")),
        "{offered:?}"
    );
    assert_eq!(
        offered.iter().filter(|(_, l)| l == "Save").count(),
        1,
        "{offered:?}"
    );
}

#[tokio::test]
async fn a_generic_clickable_is_clicked_and_a_nested_one_is_not() {
    let harness = harness_or_skip!();
    let mut page = harness.open("clickables.html").await.unwrap();
    let observation = page.observe().await.unwrap();

    let (_, observation) = act_on(&mut page, &observation, "click", "In", None)
        .await
        .unwrap();
    // The row's centre is not its star: the click lands on the row itself.
    let row = observation["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|action| {
            action["label"]
                .as_str()
                .unwrap_or_default()
                .starts_with("Bob")
        })
        .expect("the second mail row is offered")
        .clone();
    let label = row["label"].as_str().unwrap().to_string();
    let (_, observation) = act_on(&mut page, &observation, "click", &label, None)
        .await
        .unwrap();
    let star = observation["actions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|action| action["label"] == "Star")
        .nth(1)
        .expect("the second star")
        .clone();
    page.act(
        &star,
        &observation,
        None,
        std::time::Duration::from_millis(100),
    )
    .await
    .unwrap();
    page.settle().await;
    assert_eq!(
        page.evaluate("clicked").await.unwrap(),
        json!(["in", "row 2", "star 2"])
    );
    // The scripted plan names targets by kind and label, as the loop does.
    assert!(find(&observation, "click", "Prev").is_some());
}

#[tokio::test]
async fn unnamed_twins_in_a_grid_are_told_apart_by_row_and_column() {
    let harness = harness_or_skip!();
    let mut page = harness.open("board.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    let contexts = observation["actions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|action| action["kind"] == "click")
        .map(|action| action["context"].as_str().unwrap_or_default().to_string())
        .collect::<Vec<_>>();
    let mut expected = Vec::new();
    for row in 1..=3 {
        for column in 1..=3 {
            expected.push(format!("row {row}, column {column}"));
        }
    }
    // A single row is not a grid: those twins are numbered among all twins.
    expected.extend(["10 of 12", "11 of 12", "12 of 12"].map(String::from));
    assert_eq!(contexts, expected);

    let cell = observation["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|action| action["context"] == "row 2, column 3")
        .unwrap()
        .clone();
    page.act(
        &cell,
        &observation,
        None,
        std::time::Duration::from_millis(100),
    )
    .await
    .unwrap();
    page.settle().await;
    assert_eq!(
        page.evaluate("document.getElementById('status').textContent")
            .await
            .unwrap(),
        json!("Marked row 2, column 3")
    );
}
