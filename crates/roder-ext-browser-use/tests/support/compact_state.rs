//! Fixtures for the compact page-state tests: a `browser_get_state` text in
//! the shape the pinned browser-use 0.13.10 server produces, and readers for
//! the one-line-per-element view Roder turns it into.
//!
//! The server builds `{url, title, tabs, interactive_elements, viewport, page,
//! scroll}` and returns `json.dumps(result, indent=2)`; each element is
//! `{index, tag, text}` plus `placeholder` and `href` when the page has them.

use serde_json::{Value, json};

/// The element `index` of the `position`-th element. Not contiguous on purpose:
/// the offset a page ends with counts positions, never index values.
pub fn index_at(position: usize) -> u64 {
    position as u64 * 2 + 1
}

/// Every index of a page of `count` elements, in page order.
pub fn expected_indices(count: usize) -> Vec<u64> {
    (0..count).map(index_at).collect()
}

fn element(position: usize, secret: Option<&str>) -> Value {
    let index = index_at(position);
    // Every element of the secrets page is a link carrying the key in its
    // address, at a depth that grows with the position, so some of them
    // straddle any length cap on an address.
    if let Some(secret) = secret {
        return json!({
            "index": index, "tag": "a",
            "text": if position == 0 { secret.to_string() } else { format!("Link {position}") },
            "href": format!("https://example.com/{}{secret}", "p".repeat(position * 10)),
        });
    }
    match position % 4 {
        0 => json!({
            "index": index, "tag": "a",
            "text": format!("Link {position}"),
            "href": format!("https://example.com/p/{position}"),
        }),
        1 => json!({"index": index, "tag": "button", "text": format!("Button {position}")}),
        2 => json!({
            "index": index, "tag": "input", "text": "",
            "placeholder": format!("Field {position}"),
        }),
        _ => json!({
            "index": index, "tag": "div",
            "text": format!("Row {position} {}", "lorem ipsum ".repeat(7)).trim_end(),
        }),
    }
}

/// The pretty-printed state of a page with `count` elements. With `secret`
/// the title, the first link's text and the address of every link carry it.
pub fn upstream_state(url: &str, count: usize, secret: Option<&str>) -> String {
    let title = match secret {
        Some(secret) => format!("Account {secret}"),
        None => "Many elements".to_string(),
    };
    let elements: Vec<Value> = (0..count).map(|n| element(n, secret)).collect();
    serde_json::to_string_pretty(&json!({
        "url": url,
        "title": title,
        "tabs": [{"url": url, "title": title}],
        "interactive_elements": elements,
        "viewport": {"width": 1280, "height": 720},
        "page": {"width": 1280, "height": 9000},
        "scroll": {"x": 0, "y": 0},
    }))
    .unwrap()
}

/// The pretty-printed state of a small form: a link, a text field, a native
/// select (index 7), a button and a custom dropdown built from a `div` (index
/// 11). `clicks` and `types` are how many `browser_click` and `browser_type`
/// calls the server has received; they are unknown fields to the wrapper, so
/// the compact view shows them as `clicks: N` and `types: N` header lines.
pub fn form_state(url: &str, clicks: u32, types: u32, with_select: bool) -> String {
    let mut elements = vec![
        json!({"index": 3, "tag": "a", "text": "Home", "href": "/"}),
        json!({"index": 5, "tag": "input", "text": "", "placeholder": "Name"}),
    ];
    if with_select {
        elements.push(json!({
            "index": 7, "tag": "select", "text": "Pick one Alpha Banana Cherry"
        }));
    }
    elements.push(json!({"index": 9, "tag": "button", "text": "Submit"}));
    elements.push(json!({"index": 11, "tag": "div", "text": "Custom dropdown"}));
    serde_json::to_string_pretty(&json!({
        "url": url,
        "title": "Order form",
        "tabs": [{"url": url, "title": "Order form"}],
        "interactive_elements": elements,
        "viewport": {"width": 1280, "height": 720},
        "page": {"width": 1280, "height": 720},
        "scroll": {"x": 0, "y": 0},
        "clicks": clicks,
        "types": types,
    }))
    .unwrap()
}

/// The value of the `name: N` header line of a rendered state.
pub fn counter(text: &str, name: &str) -> u32 {
    text.lines()
        .find_map(|line| line.strip_prefix(&format!("{name}: "))?.parse().ok())
        .unwrap_or_else(|| panic!("no {name} counter in:\n{text}"))
}

/// A state text that is not JSON at all.
pub const PLAIN: &str = "The page is still loading. Try again in a moment.";

/// The state view inside a result: from its `url:` line up to the note about
/// the attached screenshot, if any.
pub fn view(text: &str) -> String {
    let mut lines = text.lines().skip_while(|line| !line.starts_with("url: "));
    let mut view = Vec::new();
    for line in &mut lines {
        if line.starts_with("[image/") {
            break;
        }
        view.push(line);
    }
    assert!(!view.is_empty(), "no state view in:\n{text}");
    view.join("\n")
}

/// Indexes of the element lines in `text`, in order.
pub fn listed_indices(text: &str) -> Vec<u64> {
    text.lines()
        .filter_map(|line| {
            let (index, rest) = line.strip_prefix('[')?.split_once("] ")?;
            // An element line has a tag next to the index.
            rest.chars().next().filter(|c| c.is_ascii_alphabetic())?;
            index.parse().ok()
        })
        .collect()
}

/// `(omitted, next offset)` from the line a cut view ends with.
pub fn omission(text: &str) -> Option<(usize, usize)> {
    let line = text.lines().find(|line| line.starts_with("… "))?;
    let rest = line.strip_prefix("… ")?;
    let (omitted, rest) = rest.split_once(" more interactive elements not listed.")?;
    let offset = rest
        .strip_prefix(" Call browser_use_get_state with offset ")?
        .strip_suffix('.')?;
    Some((omitted.parse().ok()?, offset.parse().ok()?))
}
