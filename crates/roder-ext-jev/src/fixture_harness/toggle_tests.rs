//! A checkbox's state in the result's controls, on real pages: the visible
//! native checkbox of `basic.html` and the styled one of `filters.html`,
//! which is offered through its label.

use std::sync::Arc;
use std::time::Duration;

use super::scripted::{PlanDecider, pick};
use crate::engine::{JevControl, JevRunResult};

fn control<'a>(result: &'a JevRunResult, label: &str) -> &'a JevControl {
    result
        .controls
        .iter()
        .find(|control| control.label == label)
        .unwrap_or_else(|| panic!("no control {label}: {:#?}", result.controls))
}

#[tokio::test]
async fn a_native_checkbox_reaches_the_result_as_its_state_before_and_after_a_click() {
    let harness = harness_or_skip!();
    let timeout = Duration::from_secs(30);

    let before = harness
        .run(
            "basic.html",
            Arc::new(PlanDecider::new(Vec::new())),
            None,
            timeout,
        )
        .await;
    let newsletter = control(&before, "Newsletter");
    // Not the HTML value, which is "on".
    assert_eq!(newsletter.checked, Some(false), "{newsletter:?}");
    assert_eq!(newsletter.value, None, "{newsletter:?}");

    let after = harness
        .run(
            "basic.html",
            Arc::new(PlanDecider::new(vec![pick("click", "Newsletter")])),
            None,
            timeout,
        )
        .await;
    let newsletter = control(&after, "Newsletter");
    assert_eq!(newsletter.checked, Some(true), "{newsletter:?}");
    assert_eq!(newsletter.value, None, "{newsletter:?}");
    // A text field on the same page keeps its value and has no state.
    let name = control(&after, "Full name");
    assert_eq!(name.checked, None, "{name:?}");
    assert_eq!(control(&after, "Save draft").checked, None);
}

#[tokio::test]
async fn a_checkbox_drawn_by_its_label_reports_its_state_too() {
    let harness = harness_or_skip!();
    let result = harness
        .run(
            "filters.html",
            Arc::new(PlanDecider::new(vec![pick("click", "Free shipping")])),
            None,
            Duration::from_secs(30),
        )
        .await;
    assert_eq!(control(&result, "Free shipping").checked, Some(true));
    assert_eq!(control(&result, "In stock only").checked, Some(false));
}
