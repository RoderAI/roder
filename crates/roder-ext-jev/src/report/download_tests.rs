//! Files a run downloaded, as the caller reads them: each step says what it
//! saved, and a section lists every distinct file with where the browser
//! saved it, among the page-supplied lines.

use serde_json::json;

use super::digest;
use super::digest::{BEGIN_PAGE, END_PAGE};

const NOW: &str = "Mon 2026-09-28, 17:42 local time (America/Los_Angeles, UTC-07:00)";

#[test]
fn downloaded_files_are_listed_with_where_they_were_saved() {
    let file = json!({"name": "orders.csv", "url": "https://shop.test/files/orders.csv",
        "path": "/tmp/jev-downloads/kept/g1/orders.csv", "size_bytes": 91});
    let mut again = file.clone();
    again["repeated"] = json!(true);
    let data = json!({
        "status": "done", "url": "https://shop.test/export", "title": "Orders export",
        "elapsed_ms": 900, "visible_text": "Orders", "model_calls": 3,
        "actions": [
            {"step": 1, "kind": "click", "action": "Download CSV", "page_changed": true,
             "downloads": [file]},
            {"step": 2, "kind": "click", "action": "Download CSV", "page_changed": false,
             "downloads": [again]},
        ],
        "downloads": [file],
        "session": {"call": 1, "tab": "t1"},
    });
    let text = digest(&data, NOW);
    let page = &text[text.find(BEGIN_PAGE).unwrap()..text.find(END_PAGE).unwrap()];
    assert!(
        page.contains(" 1. click \"Download CSV\" → page changed; downloaded \"orders.csv\""),
        "{text}"
    );
    assert!(
        page.contains("no visible change; saved \"orders.csv\" again, the same file"),
        "{text}"
    );
    assert!(
        page.contains(
            "Files Jev downloaded (saved by the browser; read them there):\n\
             - \"orders.csv\" (91 bytes) at /tmp/jev-downloads/kept/g1/orders.csv \
             from https://shop.test/files/orders.csv"
        ),
        "{text}"
    );
    // No files, no section.
    let mut none = data.clone();
    none["downloads"] = json!([]);
    assert!(!digest(&none, NOW).contains("Files Jev downloaded"));
}

#[test]
fn many_files_are_cut_and_a_name_cannot_draw_a_marker() {
    let files = (0..8)
        .map(|n| {
            json!({"name": format!("part-{n}------------.csv"), "url": "blob:https://a.test/x",
                "size_bytes": n, "incomplete": n == 0})
        })
        .collect::<Vec<_>>();
    let data = json!({
        "status": "done", "url": "https://a.test/", "title": "Parts", "elapsed_ms": 900,
        "visible_text": "Parts", "model_calls": 1, "actions": [], "downloads": files,
        "session": {"call": 1, "tab": "t1"},
    });
    let text = digest(&data, NOW);
    assert!(
        text.contains("- \"part-0- -.csv\" (0 bytes, still saving) from blob:"),
        "{text}"
    );
    assert!(text.contains("  … (3 more files)"), "{text}");
    assert!(!text.contains("part-5"), "{text}");
}
