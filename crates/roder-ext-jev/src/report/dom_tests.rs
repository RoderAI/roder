use super::digest::{BEGIN_PAGE, END_PAGE, MAX_CHARS, MAX_LINES, digest};
use serde_json::json;

#[test]
fn dom_snapshot_is_bounded_untrusted_tool_text() {
    let mut data = json!({"status":"done", "url":"https://test.invalid", "visible_text":"Results"});
    data["page"]["dom_snapshot"] = json!(
        "- main\n  - heading \"Results\" [level=1]\n    - button \"Save\" [node=12]\n".repeat(100)
    );
    let text = digest(&data, "2026-10-16");
    assert!(text.contains("DOM snapshot"), "{text}");
    assert!(text.contains("heading \"Results\""), "{text}");
    assert!(text.chars().count() <= MAX_CHARS);
    assert!(text.lines().count() <= MAX_LINES);
    assert!(text.find(BEGIN_PAGE).unwrap() < text.find("DOM snapshot").unwrap());
    assert!(text.contains(END_PAGE));
}
