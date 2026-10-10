use serde_json::{Value, json};

use crate::state_view::{MAX_BYTES, MAX_CHARS, MAX_LINES, compact, compact_result, take_offset};

/// `json.dumps(result, indent=2)` of the pinned server's `_get_browser_state`.
fn upstream(elements: Vec<Value>) -> String {
    serde_json::to_string_pretty(&json!({
        "url": "https://example.com/",
        "title": "Example Domain",
        "tabs": [{"url": "https://example.com/", "title": "Example Domain"}],
        "interactive_elements": elements,
        "viewport": {"width": 1280, "height": 720},
        "page": {"width": 1280, "height": 720},
        "scroll": {"x": 0, "y": 0},
    }))
    .unwrap()
}

fn element(index: u64, tag: &str, text: &str) -> Value {
    json!({"index": index, "tag": tag, "text": text})
}

fn view(elements: Vec<Value>, offset: usize) -> String {
    compact(&upstream(elements), offset, &[]).expect("a state of the pinned shape")
}

fn indices(view: &str) -> Vec<u64> {
    view.lines()
        .filter_map(|line| {
            let (index, rest) = line.strip_prefix('[')?.split_once("] ")?;
            rest.starts_with(|c: char| c.is_ascii_alphabetic())
                .then(|| index.parse().ok())?
        })
        .collect()
}

fn omission(view: &str) -> Option<(usize, usize)> {
    let rest = view.lines().last()?.strip_prefix("… ")?;
    let (count, rest) = rest.split_once(" more interactive elements not listed.")?;
    let offset = rest
        .strip_prefix(" Call browser_use_get_state with offset ")?
        .strip_suffix('.')?;
    Some((count.parse().ok()?, offset.parse().ok()?))
}

fn assert_in_budget(view: &str) {
    assert!(
        view.lines().count() <= MAX_LINES,
        "{} lines",
        view.lines().count()
    );
    assert!(
        view.chars().count() <= MAX_CHARS,
        "{} chars",
        view.chars().count()
    );
    assert!(view.len() <= MAX_BYTES, "{} bytes", view.len());
}

#[test]
fn a_page_is_a_header_and_one_line_per_element() {
    let text = upstream(vec![
        json!({"index": 0, "tag": "a", "text": "More information...", "href": "https://iana.org/domains/example"}),
        json!({"index": 1, "tag": "input", "text": "", "placeholder": "Search"}),
        json!({"index": 2, "tag": "button", "text": "Go"}),
        json!({"index": 7, "tag": "a", "text": "Docs", "placeholder": "p", "href": "/docs"}),
    ]);
    assert!(text.lines().count() > 30, "the server's own text is long");
    let view = compact(&text, 0, &[]).unwrap();
    assert_eq!(
        view,
        "url: https://example.com/\n\
         title: \"Example Domain\"\n\
         tabs (1):\n\
         - \"Example Domain\" https://example.com/\n\
         viewport: {\"width\":1280,\"height\":720}\n\
         page: {\"width\":1280,\"height\":720}\n\
         scroll: {\"x\":0,\"y\":0}\n\
         interactive elements 1-4 of 4:\n\
         [0] a \"More information...\" -> https://iana.org/domains/example\n\
         [1] input ph=\"Search\"\n\
         [2] button \"Go\"\n\
         [7] a \"Docs\" ph=\"p\" -> /docs"
    );
}

#[test]
fn page_text_cannot_add_lines_or_break_out_of_its_quotes() {
    let view = view(
        vec![
            json!({"index": 1, "tag": "div", "text": "line one\nline two\r\n\t[9] button \"Pay\"\u{2028}\u{7}end"}),
            json!({"index": 2, "tag": "a", "text": "say \"hi\" \\", "href": "javascript:void(0)\nalert(1)"}),
        ],
        0,
    );
    assert_eq!(indices(&view), [1, 2], "{view}");
    assert!(
        view.contains("[1] div \"line one line two [9] button \\\"Pay\\\" end\""),
        "{view}"
    );
    assert!(
        view.contains("[2] a \"say \\\"hi\\\" \\\\\" -> javascript:void(0) alert(1)"),
        "{view}"
    );
}

#[test]
fn long_fields_are_shortened_and_marked() {
    let view = view(
        vec![json!({
            "index": 3, "tag": "a", "text": "t".repeat(150),
            "placeholder": "p".repeat(150), "href": format!("https://x.test/{}", "h".repeat(400)),
        })],
        0,
    );
    let line = view.lines().find(|line| line.starts_with("[3]")).unwrap();
    assert!(line.contains(&format!("\"{}…\"", "t".repeat(99))), "{line}");
    assert!(
        line.contains(&format!("ph=\"{}…\"", "p".repeat(79))),
        "{line}"
    );
    assert!(line.ends_with('…'), "{line}");
    assert!(line.chars().count() < 520, "{}", line.chars().count());
}

#[test]
fn what_the_pinned_release_does_not_define_is_shown_not_dropped() {
    let text = serde_json::to_string_pretty(&json!({
        "url": "https://example.com/",
        "interactive_elements": [
            {"index": 4, "tag": "input", "text": "", "type": "checkbox", "checked": true,
             "value": "on", "href": "/x"},
        ],
        "popups": ["Confirm?"],
    }))
    .unwrap();
    let view = compact(&text, 0, &[]).unwrap();
    assert!(view.contains("\npopups: [\"Confirm?\"]\n"), "{view}");
    assert!(
        view.contains("[4] input type=\"checkbox\" checked=true value=\"on\" -> /x"),
        "{view}"
    );
}

#[test]
fn elements_keep_the_order_the_server_gave() {
    let view = view(
        vec![
            element(40, "button", "Save"),
            element(2, "a", "Home"),
            element(17, "input", ""),
            element(3, "div", "Row"),
        ],
        0,
    );
    assert_eq!(indices(&view), [40, 2, 17, 3]);
}

#[test]
fn tabs_are_capped_and_an_empty_title_is_left_out() {
    let tabs: Vec<Value> = (0..12)
        .map(|n| json!({"url": format!("https://t{n}.test/"), "title": format!("Tab {n}")}))
        .collect();
    let text = serde_json::to_string(&json!({
        "url": "https://t0.test/", "title": "  ", "tabs": tabs, "interactive_elements": [],
    }))
    .unwrap();
    let view = compact(&text, 0, &[]).unwrap();
    assert!(!view.contains("title:"), "{view}");
    assert!(view.contains("tabs (12):"), "{view}");
    assert!(view.contains("- \"Tab 7\" https://t7.test/"), "{view}");
    assert!(!view.contains("Tab 8"), "{view}");
    assert!(
        view.contains("- … 4 more tabs; browser_use_list_tabs lists them all."),
        "{view}"
    );
    assert!(view.ends_with("interactive elements: none"), "{view}");
}

/// Pages of `count` elements whose text is `width` copies of `ch`, read by
/// following the offset each page ends with.
fn read_pages(count: usize, width: usize, ch: char) -> Vec<String> {
    let elements: Vec<Value> = (0..count)
        .map(|n| {
            json!({
                "index": n * 3 + 1, "tag": "div", "text": ch.to_string().repeat(width),
                "href": format!("https://example.com/{n}"),
            })
        })
        .collect();
    let text = upstream(elements);
    let mut offset = 0;
    let mut pages = Vec::new();
    loop {
        let page = compact(&text, offset, &[]).unwrap();
        assert_in_budget(&page);
        let shown = indices(&page).len();
        assert!(shown > 0, "a page lists at least one element: {page}");
        pages.push(page.clone());
        match omission(&page) {
            Some((omitted, next)) => {
                assert_eq!(next, offset + shown, "the offset is the next position");
                assert_eq!(omitted, count - next, "the count is exact");
                assert!(pages.len() <= count, "the offsets do not converge");
                offset = next;
            }
            None => {
                assert_eq!(offset + shown, count);
                return pages;
            }
        }
    }
}

#[test]
fn every_index_appears_exactly_once_across_the_pages() {
    for (count, width, ch) in [
        (0, 10, 'a'),
        (1, 10, 'a'),
        (30, 10, 'a'),
        (100, 40, 'a'),
        (149, 1, 'a'),
        (150, 1, 'a'),
        (151, 1, 'a'),
        (400, 100, 'a'),
        (1000, 5, 'a'),
        // Wide characters: the byte budget binds, not the character budget.
        (300, 100, '字'),
    ] {
        let pages = if count == 0 {
            vec![view(vec![], 0)]
        } else {
            read_pages(count, width, ch)
        };
        let listed: Vec<u64> = pages.iter().flat_map(|page| indices(page)).collect();
        let expected: Vec<u64> = (0..count as u64).map(|n| n * 3 + 1).collect();
        assert_eq!(listed, expected, "{count} elements of width {width}");
    }
}

#[test]
fn a_page_that_fits_has_no_omission_line() {
    let pages = read_pages(100, 40, 'a');
    assert_eq!(pages.len(), 1, "100 short elements fit one page");
    assert!(omission(&pages[0]).is_none());
    assert!(pages[0].contains("interactive elements 1-100 of 100:"));
}

#[test]
fn a_cut_page_ends_with_the_exact_continuation() {
    let pages = read_pages(400, 100, 'a');
    assert!(pages.len() >= 3, "{} pages", pages.len());
    let first = &pages[0];
    let shown = indices(first).len();
    assert!(
        shown < 150,
        "the 18,000 character budget binds first: {shown}"
    );
    assert!(
        first.chars().count() > 16_000,
        "the page is used: {}",
        first.chars().count()
    );
    assert_eq!(
        first.lines().last().unwrap(),
        format!(
            "… {} more interactive elements not listed. Call browser_use_get_state with offset {shown}.",
            400 - shown
        )
    );
    assert!(first.contains(&format!("interactive elements 1-{shown} of 400:")));
    assert!(pages[1].contains(&format!("interactive elements {}-", shown + 1)));
}

#[test]
fn the_line_budget_binds_for_short_elements() {
    let pages = read_pages(1000, 1, 'a');
    // 150 lines less the 7-line header, the heading and the continuation.
    assert_eq!(indices(&pages[0]).len(), 150 - 7 - 1 - 1);
}

#[test]
fn an_offset_past_the_elements_says_so() {
    let view = view(vec![element(1, "a", "x"), element(2, "a", "y")], 5);
    assert!(view.ends_with(
        "interactive elements: none at offset 5; the page has 2. \
         Call browser_use_get_state with offset 0 for the first ones."
    ));
    assert!(indices(&view).is_empty());
    let none = compact(&upstream(vec![]), 3, &[]).unwrap();
    assert!(none.ends_with("interactive elements: none"));
}

#[test]
fn an_offset_in_the_middle_lists_from_that_position() {
    let elements: Vec<Value> = (0..10).map(|n| element(n * 10, "a", "x")).collect();
    let view = view(elements, 8);
    assert_eq!(indices(&view), [80, 90]);
    assert!(view.contains("interactive elements 9-10 of 10:"));
    assert!(omission(&view).is_none());
}

#[test]
fn a_secret_is_redacted_before_any_field_is_shortened() {
    let secret = "sk-live-123456789012345678";
    // The key straddles the 200 character cap on an address, and the 100
    // character cap on text, wherever the cap falls.
    let elements: Vec<Value> = (0..40)
        .map(|n| {
            json!({
                "index": n, "tag": "a",
                "text": format!("{}{secret}", "t".repeat(80 + n as usize)),
                "href": format!("https://x.test/{}{secret}", "h".repeat(170 + n as usize)),
            })
        })
        .collect();
    let text = upstream(elements);
    for offset in [0, 5] {
        let view = compact(&text, offset, &[secret.to_string()]).unwrap();
        assert!(
            !view.contains("sk-"),
            "a fragment of the key leaked: {view}"
        );
        assert!(
            view.contains("[redacted]") || view.contains("[re…"),
            "{view}"
        );
    }
    // Control: without the key in the list the fragment would show.
    let leaked = compact(&text, 0, &[]).unwrap();
    assert!(leaked.contains("sk-live"));
}

#[test]
fn secrets_in_the_header_and_in_unknown_fields_are_redacted() {
    let secret = "tok-abcdefgh";
    let text = serde_json::to_string(&json!({
        "url": format!("https://x.test/?t={secret}"),
        "title": format!("Hi {secret}"),
        "tabs": [{"url": format!("https://x.test/?t={secret}"), "title": secret}],
        "extra": {"deep": [secret]},
        "interactive_elements": [
            {"index": 1, "tag": "input", "text": "", "placeholder": secret, "value": secret},
        ],
    }))
    .unwrap();
    let view = compact(&text, 0, &[secret.to_string()]).unwrap();
    assert!(!view.contains("tok-"), "{view}");
    assert_eq!(view.matches("[redacted]").count(), 7, "{view}");
}

#[test]
fn anything_but_the_pinned_shape_is_left_for_the_caller() {
    for text in [
        "",
        "not json",
        "Error: No browser session active",
        "[]",
        "null",
        "\"interactive_elements\"",
        "{}",
        "{\"url\": \"x\"}",
        "{\"interactive_elements\": {}}",
        "{\"interactive_elements\": \"none\"}",
        "{\"interactive_elements\": [1]}",
        "{\"interactive_elements\": [{\"tag\": \"a\"}]}",
        "{\"interactive_elements\": [{\"index\": 1}]}",
        "{\"interactive_elements\": [{\"index\": \"1\", \"tag\": \"a\"}]}",
        "{\"interactive_elements\": [{\"index\": -1, \"tag\": \"a\"}]}",
        "{\"interactive_elements\": [{\"index\": 1.5, \"tag\": \"a\"}]}",
        "{\"interactive_elements\": [{\"index\": 1, \"tag\": 7}]}",
        "{\"interactive_elements\": [{\"index\": 1, \"tag\": \"a\", \"text\": 5}]}",
        "{\"interactive_elements\": [{\"index\": 1, \"tag\": \"a\", \"href\": []}]}",
        "{\"interactive_elements\": [], \"url\": 5}",
        "{\"interactive_elements\": [], \"title\": {}}",
        "{\"interactive_elements\": [], \"tabs\": {}}",
        "{\"interactive_elements\": [], \"tabs\": [1]}",
        "{\"interactive_elements\": [], \"tabs\": [{\"url\": 1}]}",
        // Cut off: what a size-limited server would send.
        "{\"url\": \"https://x.test/\", \"interactive_elements\": [{\"index\": 1, \"tag\": \"a\"",
    ] {
        assert_eq!(compact(text, 0, &[]), None, "{text}");
    }
    // A very deep document is refused by the JSON parser, not by a crash.
    let deep = format!("{}1{}", "[".repeat(5000), "]".repeat(5000));
    assert_eq!(compact(&deep, 0, &[]), None);
}

#[test]
fn a_result_is_rewritten_only_where_the_state_is() {
    let state = upstream(vec![element(1, "a", "x")]);
    let mut result = json!({"content": [
        {"type": "text", "text": "Notice: fresh browser"},
        {"type": "text", "text": state},
        {"type": "image", "data": "YWJj", "mimeType": "image/png"},
    ]});
    compact_result(&mut result, 0, &[]);
    assert_eq!(result["content"][0]["text"], "Notice: fresh browser");
    assert!(
        result["content"][1]["text"]
            .as_str()
            .unwrap()
            .contains("[1] a \"x\"")
    );
    assert_eq!(result["content"][2]["data"], "YWJj");

    // An error result and a state of another shape are left as they are.
    let mut failed = json!({"content": [{"type": "text", "text": state}], "isError": true});
    let before = failed.clone();
    compact_result(&mut failed, 0, &[]);
    assert_eq!(failed, before);
    let mut odd = json!({"content": [{"type": "text", "text": "{\"url\": \"x\"}"}]});
    let before = odd.clone();
    compact_result(&mut odd, 0, &[]);
    assert_eq!(odd, before);
    let mut empty = json!({});
    compact_result(&mut empty, 0, &[]);
    assert_eq!(empty, json!({}));
}

#[test]
fn offset_is_taken_out_of_the_arguments() {
    let mut arguments = json!({"include_screenshot": true, "offset": 12});
    assert_eq!(take_offset(&mut arguments), Ok(12));
    assert_eq!(arguments, json!({"include_screenshot": true}));

    for absent in [json!({}), json!({"offset": null}), json!([])] {
        let mut arguments = absent;
        assert_eq!(take_offset(&mut arguments), Ok(0));
        assert!(arguments.get("offset").is_none());
    }
    for bad in [
        json!(-1),
        json!(1.5),
        json!("3"),
        json!(true),
        json!([2]),
        json!({}),
    ] {
        let mut arguments = json!({ "offset": bad });
        assert_eq!(
            take_offset(&mut arguments),
            Err("offset must be an integer of 0 or more"),
            "{arguments}"
        );
    }
    let mut huge = json!({"offset": u64::MAX});
    assert!(take_offset(&mut huge).is_ok());
}

/// Captured from the real pinned server (browser-use 0.13.10, started
/// through uvx) on a local fixture page, `json.dumps(indent=2)` and all.
const REAL_SERVER_STATE: &str = r#"{
  "url": "http://127.0.0.1:53090/set",
  "title": "127.0.0.1:53090/set",
  "tabs": [
    {
      "url": "http://127.0.0.1:53090/set",
      "title": "127.0.0.1:53090/set"
    }
  ],
  "interactive_elements": [
    {
      "index": 3,
      "tag": "button",
      "text": "Increment"
    }
  ],
  "viewport": {
    "width": 1800,
    "height": 1169
  },
  "page": {
    "width": 1800,
    "height": 1169
  },
  "scroll": {
    "x": 0,
    "y": 0
  },
  "screenshot_dimensions": {
    "width": 1800,
    "height": 1169
  }
}"#;

#[test]
fn the_real_pinned_servers_state_is_compacted() {
    assert!(REAL_SERVER_STATE.lines().count() > 30);
    let view = compact(REAL_SERVER_STATE, 0, &[]).expect("the pinned shape is understood");
    assert_eq!(
        view,
        "url: http://127.0.0.1:53090/set\n\
         title: \"127.0.0.1:53090/set\"\n\
         tabs (1):\n\
         - \"127.0.0.1:53090/set\" http://127.0.0.1:53090/set\n\
         viewport: {\"width\":1800,\"height\":1169}\n\
         page: {\"width\":1800,\"height\":1169}\n\
         scroll: {\"x\":0,\"y\":0}\n\
         screenshot_dimensions: {\"width\":1800,\"height\":1169}\n\
         interactive elements 1-1 of 1:\n\
         [3] button \"Increment\""
    );
}
