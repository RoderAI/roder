//! The compact page state: budgets, paging, pass-through and redaction.

use roder_api::tools::ToolRegistry;
use roder_ext_browser_use::{BrowserUseConfig, UNTRUSTED_NOTE};
use serde_json::json;

use crate::compact_state::{self, expected_indices, listed_indices, omission, upstream_state};
use crate::support::{OPENAI_KEY, fake_launch, keyed_config, registry_for, run};

/// What core accepts in a tool result before it swaps it for an excerpt.
const CORE_MAX_LINES: usize = 200;
const CORE_MAX_CHARS: usize = 20_000;

/// The state view in `text` fits its own budget, and the whole result with the
/// envelope and report still fits core's caps.
fn assert_within_budget(text: &str) {
    let lines = text.lines().count();
    let chars = text.chars().count();
    assert!(
        lines <= CORE_MAX_LINES && chars <= CORE_MAX_CHARS,
        "{lines} lines, {chars} chars would be spilled by core:\n{}",
        &text[..text.len().min(600)]
    );
    let view = compact_state::view(text);
    assert!(
        view.lines().count() <= 150,
        "{} view lines",
        view.lines().count()
    );
    assert!(
        view.chars().count() <= 18_000,
        "{} view chars",
        view.chars().count()
    );
}

/// Reads every page of `get_state` by following the offset each one ends
/// with. Returns the indexes in the order listed, and the number of pages.
async fn read_all_pages(registry: &ToolRegistry) -> (Vec<u64>, usize) {
    let mut listed = Vec::new();
    let mut offset = 0;
    for pages in 1.. {
        let page = run(registry, "browser_use_get_state", json!({"offset": offset})).await;
        assert!(!page.is_error, "{}", page.text);
        assert!(
            !page.text.contains("unexpected argument"),
            "offset reached the server: {}",
            page.text
        );
        assert_within_budget(&page.text);
        let shown = listed_indices(&page.text);
        assert!(!shown.is_empty(), "a page with no elements: {}", page.text);
        listed.extend(&shown);
        match omission(&page.text) {
            Some((omitted, next)) => {
                assert_eq!(next, offset + shown.len(), "{}", page.text);
                assert!(omitted > 0);
                assert!(pages < 20, "the offsets do not converge");
                offset = next;
            }
            None => return (listed, pages),
        }
    }
    unreachable!()
}

#[tokio::test]
async fn a_100_element_state_is_one_line_per_element_and_every_index_appears_once() {
    // The fake speaks the pinned server's pretty-printed shape, which is far
    // past core's 200-line cap for this page.
    let raw = upstream_state("https://example.com/elements/100", 100, None);
    assert!(raw.lines().count() > 500, "{} lines", raw.lines().count());

    let server = fake_launch(BrowserUseConfig::default(), None);
    let registry = registry_for(server.clone(), false);
    let navigated = run(
        &registry,
        "browser_use_navigate",
        json!({"url": "https://example.com/elements/100"}),
    )
    .await;
    assert!(!navigated.is_error, "{}", navigated.text);
    assert_within_budget(&navigated.text);
    assert!(
        navigated
            .text
            .contains("Navigated to: https://example.com/elements/100"),
        "{}",
        &navigated.text[..400]
    );
    // The state that follows an action uses the same view.
    assert!(!navigated.text.contains("\"interactive_elements\""));
    assert!(
        navigated
            .text
            .contains("url: https://example.com/elements/100")
    );
    assert!(navigated.text.contains("title: \"Many elements\""));
    for line in [
        "[1] a \"Link 0\" -> https://example.com/p/0",
        "[3] button \"Button 1\"",
        "[5] input ph=\"Field 2\"",
    ] {
        assert!(
            navigated.text.lines().any(|l| l == line),
            "missing {line}:\n{}",
            navigated.text
        );
    }

    let (listed, pages) = read_all_pages(&registry).await;
    assert!(pages >= 1);
    // Page order, none reordered, none repeated, none lost.
    assert_eq!(listed, expected_indices(100));
    server.shutdown().await;
}

#[tokio::test]
async fn a_400_element_state_keeps_the_report_and_counts_what_it_leaves_out() {
    let server = fake_launch(BrowserUseConfig::default(), None);
    let registry = registry_for(server.clone(), false);
    let navigated = run(
        &registry,
        "browser_use_navigate",
        json!({"url": "https://example.com/elements/400"}),
    )
    .await;
    assert!(!navigated.is_error, "{}", navigated.text);
    assert_within_budget(&navigated.text);

    let report = navigated
        .text
        .find("Navigated to: https://example.com/elements/400")
        .expect("the report survives");
    let view = navigated
        .text
        .find("interactive elements 1-")
        .expect("view");
    assert!(report < view);
    assert!(
        navigated.text.contains("screenshot attached"),
        "the screenshot note survives"
    );
    let shown = listed_indices(&navigated.text);
    let (omitted, next) = omission(&navigated.text).expect("the cut says how much it left out");
    assert_eq!(shown.len() + omitted, 400);
    assert_eq!(next, shown.len());
    assert!(navigated.text.contains(&format!(
        "… {omitted} more interactive elements not listed. Call browser_use_get_state with offset {next}."
    )));
    assert!(
        !navigated.text.contains("truncated"),
        "the byte cut did not need to fire"
    );
    assert_eq!(navigated.data["__view_image"]["detail"], "original");

    let (listed, pages) = read_all_pages(&registry).await;
    assert!(pages >= 3, "400 elements cannot fit in {pages} pages");
    assert_eq!(listed, expected_indices(400));
    // The last page has nothing left to point at.
    let last = run(
        &registry,
        "browser_use_get_state",
        json!({"offset": 400 - 3}),
    )
    .await;
    assert_eq!(listed_indices(&last.text), expected_indices(400)[397..]);
    assert!(omission(&last.text).is_none(), "{}", last.text);
    server.shutdown().await;
}

#[tokio::test]
async fn offset_is_handled_by_the_wrapper_and_checked() {
    let server = fake_launch(BrowserUseConfig::default(), None);
    let registry = registry_for(server.clone(), false);
    run(
        &registry,
        "browser_use_navigate",
        json!({"url": "https://example.com/elements/100"}),
    )
    .await;

    let skip = run(&registry, "browser_use_get_state", json!({"offset": 98})).await;
    assert!(!skip.is_error, "{}", skip.text);
    assert!(!skip.text.contains("unexpected argument"), "{}", skip.text);
    assert_eq!(listed_indices(&skip.text), expected_indices(100)[98..]);
    assert!(skip.text.contains("interactive elements 99-100 of 100"));

    // Past the end is information, not an error.
    let past = run(&registry, "browser_use_get_state", json!({"offset": 5000})).await;
    assert!(!past.is_error, "{}", past.text);
    assert!(past.text.contains("the page has 100"), "{}", past.text);
    assert!(listed_indices(&past.text).is_empty());

    // Null is an omitted argument.
    let null = run(&registry, "browser_use_get_state", json!({"offset": null})).await;
    assert!(!null.is_error);
    assert_eq!(listed_indices(&null.text).first(), Some(&1));

    // A state the wrapper cannot read comes back raw, and the offset still
    // stays on this side.
    run(
        &registry,
        "browser_use_navigate",
        json!({"url": "https://example.com/odd"}),
    )
    .await;
    let odd = run(&registry, "browser_use_get_state", json!({"offset": 7})).await;
    assert!(!odd.is_error, "{}", odd.text);
    assert!(odd.text.contains("\"elements\": ["), "{}", odd.text);
    assert!(!odd.text.contains("unexpected argument"), "{}", odd.text);

    for bad in [json!(-1), json!(1.5), json!("3"), json!(true)] {
        let result = run(&registry, "browser_use_get_state", json!({"offset": bad})).await;
        assert!(result.is_error, "{bad}: {}", result.text);
        assert!(result.text.contains("offset"), "{}", result.text);
    }

    // The schema the model sees names it.
    let schema = registry
        .get("browser_use_get_state")
        .unwrap()
        .spec()
        .parameters;
    assert_eq!(schema["properties"]["offset"]["type"], "integer");
    assert_eq!(schema["properties"]["offset"]["minimum"], 0);
    assert_eq!(schema["properties"]["offset"]["default"], 0);
    assert!(schema["properties"]["include_screenshot"].is_object());
    server.shutdown().await;
}

#[tokio::test]
async fn a_state_that_is_not_the_pinned_shape_passes_through_unchanged() {
    let server = fake_launch(BrowserUseConfig::default(), None);
    let registry = registry_for(server.clone(), false);

    run(
        &registry,
        "browser_use_navigate",
        json!({"url": "https://example.com/plain"}),
    )
    .await;
    let plain = run(&registry, "browser_use_get_state", json!({})).await;
    assert!(!plain.is_error, "{}", plain.text);
    assert_eq!(
        plain.text,
        format!("{UNTRUSTED_NOTE}\n---\n{}", compact_state::PLAIN)
    );
    assert_eq!(plain.data["content"], compact_state::PLAIN);

    let odd = run(
        &registry,
        "browser_use_navigate",
        json!({"url": "https://example.com/odd"}),
    )
    .await;
    let raw = serde_json::to_string_pretty(&json!({
        "url": "https://example.com/odd", "elements": [{"id": 1, "label": "Go"}]
    }))
    .unwrap();
    assert!(
        odd.text.contains(&raw),
        "the observation is the raw state: {}",
        odd.text
    );
    let state = run(&registry, "browser_use_get_state", json!({})).await;
    assert_eq!(state.text, format!("{UNTRUSTED_NOTE}\n---\n{raw}"));
    server.shutdown().await;
}

#[tokio::test]
async fn secrets_are_redacted_before_the_view_is_cut() {
    let server = fake_launch(keyed_config(), None);
    let registry = registry_for(server.clone(), true);
    let navigated = run(
        &registry,
        "browser_use_navigate",
        json!({"url": "https://example.com/secrets"}),
    )
    .await;
    let mut texts = vec![navigated.text, navigated.data.to_string()];
    for offset in [0, 10, 20] {
        let page = run(
            &registry,
            "browser_use_get_state",
            json!({"offset": offset}),
        )
        .await;
        assert!(!page.is_error, "{}", page.text);
        texts.push(page.text);
        texts.push(page.data.to_string());
    }
    for text in &texts {
        // Not the key, and not the fragment an address cap would leave of it.
        assert!(
            !text.contains("sk-fake"),
            "a fragment of the key leaked:\n{text}"
        );
        assert!(!text.contains(OPENAI_KEY));
    }
    assert!(
        texts[0].contains("title: \"Account [redacted]\""),
        "{}",
        texts[0]
    );
    assert!(texts[0].contains("[1] a \"[redacted]\""), "{}", texts[0]);
    assert!(texts[0].contains("[redacted]"));
    server.shutdown().await;
}
