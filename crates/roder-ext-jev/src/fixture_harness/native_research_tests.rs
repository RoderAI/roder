//! Native document reads use the same rendered-text extractor as direct tools.
use super::scripted::{FieldValues, PlanDecider, pick};
use crate::JevStatus;
use std::{sync::Arc, time::Duration};
#[tokio::test]
async fn native_document_read_scrubs_an_echoed_password_before_retention() {
    let harness = harness_or_skip!();
    let plan = vec![
        pick("fill", "Password"),
        pick("click", "Show what I typed"),
        pick(
            "read_document",
            "Read rendered document text and source links without scrolling",
        ),
    ];
    let result = harness
        .run(
            "secret-echo.html",
            Arc::new(PlanDecider::new(plan)),
            Some(Arc::new(FieldValues::new(&[(
                "Password",
                "private-research-token",
            )]))),
            Duration::from_secs(15),
        )
        .await;
    assert_eq!(result.status, JevStatus::Done, "{result:#?}");
    let saved = serde_json::to_string(&result).unwrap();
    assert!(!saved.contains("private-research-token"));
    assert!(
        result.evidence.contains("You typed: [secret]"),
        "{}",
        result.evidence
    );
    assert_eq!(
        result
            .actions
            .iter()
            .filter(|a| a.kind == "read_document")
            .count(),
        1
    );
}

#[tokio::test]
async fn research_document_identity_does_not_confuse_changing_text_with_navigation() {
    use crate::JevBrowser;
    let harness = harness_or_skip!();
    let mut page = harness.open("basic.html").await.unwrap();
    let observation = page.observe().await.unwrap();
    page.evaluate("document.querySelector('h1').textContent='Updated visible fact'; true")
        .await
        .unwrap();
    assert!(!page.fresh(&observation, None).await.unwrap());
    assert!(page.fresh_document(&observation).await.unwrap());
    page.goto(&harness.site.url("landing.html")).await.unwrap();
    assert!(!page.fresh_document(&observation).await.unwrap());
    page.close().await.unwrap();
}

#[tokio::test]
async fn document_reads_put_main_first_and_keep_navigation_and_footer() {
    use crate::JevBrowser;
    let harness = harness_or_skip!();
    let mut page = harness.open("basic.html").await.unwrap();
    page.evaluate("document.body.innerHTML='<nav>Navigation text</nav><main>Main research fact</main><footer>Footer caveat</footer>'; true").await.unwrap();
    let read = page.document_text().await.unwrap().unwrap();
    let text = read["text"].as_str().unwrap();
    assert!(text.starts_with("Main research fact"), "{text}");
    assert!(text.contains("Navigation text") && text.contains("Footer caveat"));
    assert_eq!(text.matches("Main research fact").count(), 1);
    page.close().await.unwrap();
}
