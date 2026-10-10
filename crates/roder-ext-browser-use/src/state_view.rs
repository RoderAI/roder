//! The page state the model reads: a header and one line per interactive
//! element, cut to a budget that says what it left out.
//!
//! The pinned server answers `browser_get_state` with `json.dumps(indent=2)`,
//! five to seven lines for every element. A page of thirty elements already
//! passes the 200 lines core allows in a tool result, and core then replaces
//! the whole result with a head-and-tail excerpt that hides most indexes. This
//! view lists `[index] tag "text" ph="placeholder" -> href` on one line, in
//! the order the server gave, and stops before core's caps (150 lines and
//! 18,000 characters). What it leaves out is counted, and the last line says
//! which `offset` of `browser_use_get_state` continues from there. The offset
//! is handled here and never sent to the server.
//!
//! Elements are never reordered or promoted by role: QuickE2E re-ranked its
//! menu by role and lost a task it passed five times in five.
//!
//! The view fails open. A text that is not JSON, or JSON that is not the
//! shape of the pinned release, is left exactly as the server wrote it, and
//! the generic size cut applies to it as before. Secrets are redacted from
//! every string before anything is shortened, so a cut cannot leave a
//! fragment of one.
//!
//! The same parse reports which of the page's indexes are native selects, all
//! of them and not only the ones that fit the view. The wrapper keeps that set
//! for the thread (see `select_guard`).

use std::collections::HashSet;

use roder_ext_mcp::redact_secrets;
use serde_json::{Map, Value};

/// The server tool this view is made from.
pub(crate) const GET_STATE_REMOTE: &str = "browser_get_state";

/// Budget for one rendered state. Core spills a result over 200 lines or
/// 20,000 characters; the report, labels and screenshot note around the view
/// need the rest.
pub(crate) const MAX_LINES: usize = 150;
pub(crate) const MAX_CHARS: usize = 18_000;
/// The wrapper's own size cut counts bytes. Staying under it keeps the line
/// that points at the next page from being cut off a page of wide characters.
pub(crate) const MAX_BYTES: usize = 20_000;

const MAX_TABS: usize = 8;
const URL_CHARS: usize = 500;
const TITLE_CHARS: usize = 200;
const TAB_TITLE_CHARS: usize = 80;
const TAB_URL_CHARS: usize = 200;
const TAG_CHARS: usize = 24;
const TEXT_CHARS: usize = 100;
const PLACEHOLDER_CHARS: usize = 80;
const HREF_CHARS: usize = 200;
const EXTRA_CHARS: usize = 60;
const HEADER_EXTRA_CHARS: usize = 200;

/// Removes `offset` from the arguments of `browser_use_get_state` and returns
/// it. It is the number of elements to skip, 0 when absent or null.
pub(crate) fn take_offset(arguments: &mut Value) -> Result<usize, &'static str> {
    let Some(object) = arguments.as_object_mut() else {
        return Ok(0);
    };
    match object.remove("offset") {
        None | Some(Value::Null) => Ok(0),
        Some(value) => value
            .as_u64()
            .map(|offset| usize::try_from(offset).unwrap_or(usize::MAX))
            .ok_or("offset must be an integer of 0 or more"),
    }
}

/// The indexes of the native selects in a page state.
pub(crate) type Selects = HashSet<u64>;

/// Replaces the state text of a raw `browser_get_state` result with the
/// compact view of the elements from `offset` on, and returns the page's
/// native selects. Anything that is not a state of the pinned shape is left
/// alone and yields `None`: nothing is known about its selects.
pub(crate) fn compact_result(
    result: &mut Value,
    offset: usize,
    secrets: &[String],
) -> Option<Selects> {
    if result.get("isError").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let mut selects = None;
    for item in result.get_mut("content")?.as_array_mut()? {
        if item.get("type").and_then(Value::as_str) != Some("text") {
            continue;
        }
        let shown = item
            .get("text")
            .and_then(Value::as_str)
            .and_then(|text| show(text, offset, secrets));
        if let Some(shown) = shown {
            item["text"] = Value::String(shown.view);
            selects = Some(shown.selects);
        }
    }
    selects
}

/// What a page state turns into: the view the model reads and the indexes of
/// the page's native selects.
pub(crate) struct Shown {
    pub(crate) view: String,
    pub(crate) selects: Selects,
}

/// `text` as the compact view, or `None` when it is not a state of the pinned
/// shape.
pub(crate) fn show(text: &str, offset: usize, secrets: &[String]) -> Option<Shown> {
    let mut value: Value = serde_json::from_str(text).ok()?;
    redact_value(&mut value, secrets);
    let page = Page::parse(&value)?;
    Some(Shown {
        view: page.render(offset),
        selects: page.selects(),
    })
}

/// The compact view of `text`.
#[cfg(test)]
pub(crate) fn compact(text: &str, offset: usize, secrets: &[String]) -> Option<String> {
    show(text, offset, secrets).map(|shown| shown.view)
}

/// Redacts every string, so that no later cut can split a secret.
fn redact_value(value: &mut Value, secrets: &[String]) {
    match value {
        Value::String(text) => *text = redact_secrets(text, secrets),
        Value::Array(items) => items
            .iter_mut()
            .for_each(|item| redact_value(item, secrets)),
        Value::Object(map) => map
            .values_mut()
            .for_each(|item| redact_value(item, secrets)),
        _ => {}
    }
}

struct Page<'a> {
    url: Option<&'a str>,
    title: Option<&'a str>,
    /// `(title, url)` of each open tab.
    tabs: Vec<(Option<&'a str>, Option<&'a str>)>,
    /// Everything else the server reported (viewport, scroll, ...), shown as
    /// it came rather than dropped.
    extras: Vec<(&'a str, &'a Value)>,
    elements: Vec<Element<'a>>,
}

struct Element<'a> {
    index: u64,
    tag: &'a str,
    text: &'a str,
    placeholder: Option<&'a str>,
    href: Option<&'a str>,
    /// Attributes beyond the pinned release's (input type, value, ...).
    extras: Vec<(&'a str, &'a Value)>,
}

/// `Some(None)` when `key` is absent or null, `Some(Some(_))` for a string,
/// `None` for anything else.
fn string_field<'a>(object: &'a Map<String, Value>, key: &str) -> Option<Option<&'a str>> {
    match object.get(key) {
        None | Some(Value::Null) => Some(None),
        Some(Value::String(text)) => Some(Some(text)),
        Some(_) => None,
    }
}

fn extras_of<'a>(object: &'a Map<String, Value>, known: &[&str]) -> Vec<(&'a str, &'a Value)> {
    object
        .iter()
        .filter(|(key, _)| !known.contains(&key.as_str()))
        .map(|(key, value)| (key.as_str(), value))
        .collect()
}

impl<'a> Page<'a> {
    fn parse(value: &'a Value) -> Option<Self> {
        const KNOWN: &[&str] = &["url", "title", "tabs", "interactive_elements"];
        let object = value.as_object()?;
        let elements = object
            .get("interactive_elements")?
            .as_array()?
            .iter()
            .map(Element::parse)
            .collect::<Option<Vec<_>>>()?;
        let tabs = match object.get("tabs") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(tabs)) => tabs
                .iter()
                .map(|tab| {
                    let tab = tab.as_object()?;
                    Some((string_field(tab, "title")?, string_field(tab, "url")?))
                })
                .collect::<Option<Vec<_>>>()?,
            Some(_) => return None,
        };
        Some(Self {
            url: string_field(object, "url")?,
            title: string_field(object, "title")?,
            tabs,
            extras: extras_of(object, KNOWN),
            elements,
        })
    }

    /// Every native select on the page, listed or not.
    fn selects(&self) -> Selects {
        self.elements
            .iter()
            .filter(|element| element.tag.eq_ignore_ascii_case("select"))
            .map(|element| element.index)
            .collect()
    }

    fn header(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(url) = self.url {
            lines.push(format!("url: {}", capped(url, URL_CHARS)));
        }
        if let Some(title) = self.title.filter(|title| !title.trim().is_empty()) {
            lines.push(format!("title: {}", quoted(title, TITLE_CHARS)));
        }
        if !self.tabs.is_empty() {
            lines.push(format!("tabs ({}):", self.tabs.len()));
            for (title, url) in self.tabs.iter().take(MAX_TABS) {
                lines.push(format!(
                    "- {} {}",
                    quoted(title.unwrap_or(""), TAB_TITLE_CHARS),
                    capped(url.unwrap_or(""), TAB_URL_CHARS)
                ));
            }
            if self.tabs.len() > MAX_TABS {
                lines.push(format!(
                    "- … {} more tabs; browser_use_list_tabs lists them all.",
                    self.tabs.len() - MAX_TABS
                ));
            }
        }
        for (key, value) in &self.extras {
            lines.push(format!(
                "{}: {}",
                name(key),
                value_text(value, HEADER_EXTRA_CHARS)
            ));
        }
        lines
    }

    /// The header and the elements from `offset` on, within budget.
    fn render(&self, offset: usize) -> String {
        let total = self.elements.len();
        let mut lines = self.header();
        if total == 0 {
            lines.push("interactive elements: none".into());
            return lines.join("\n");
        }
        if offset >= total {
            lines.push(format!(
                "interactive elements: none at offset {offset}; the page has {total}. \
                 Call browser_use_get_state with offset 0 for the first ones."
            ));
            return lines.join("\n");
        }
        // The widest the heading can get, so that it is always paid for.
        let mut used = Size::of(&lines) + Size::line(&heading(offset, total - offset, total));
        let mut shown = Vec::new();
        for (position, element) in self.elements.iter().enumerate().skip(offset) {
            let line = element.line();
            let after = total - position - 1;
            let mut next = used + Size::line(&line);
            if after > 0 {
                next = next + Size::line(&omitted(after, position + 1));
            }
            // One element is always listed, so an offset always advances.
            if !next.fits() && !shown.is_empty() {
                break;
            }
            used = used + Size::line(&line);
            shown.push(line);
        }
        lines.push(heading(offset, shown.len(), total));
        let listed = offset + shown.len();
        lines.append(&mut shown);
        if listed < total {
            lines.push(omitted(total - listed, listed));
        }
        lines.join("\n")
    }
}

impl<'a> Element<'a> {
    fn parse(value: &'a Value) -> Option<Self> {
        const KNOWN: &[&str] = &["index", "tag", "text", "placeholder", "href"];
        let object = value.as_object()?;
        Some(Self {
            index: object.get("index")?.as_u64()?,
            tag: object.get("tag")?.as_str()?,
            text: string_field(object, "text")?.unwrap_or(""),
            placeholder: string_field(object, "placeholder")?,
            href: string_field(object, "href")?,
            extras: extras_of(object, KNOWN),
        })
    }

    /// `[index] tag "text" ph="placeholder" key=value -> href`, always one line.
    fn line(&self) -> String {
        let mut line = format!("[{}] {}", self.index, capped(self.tag, TAG_CHARS));
        if !one_line(self.text).is_empty() {
            line.push(' ');
            line.push_str(&quoted(self.text, TEXT_CHARS));
        }
        if let Some(placeholder) = self.placeholder.filter(|text| !one_line(text).is_empty()) {
            line.push_str(" ph=");
            line.push_str(&quoted(placeholder, PLACEHOLDER_CHARS));
        }
        for (key, value) in &self.extras {
            line.push_str(&format!(
                " {}={}",
                name(key),
                value_text(value, EXTRA_CHARS)
            ));
        }
        if let Some(href) = self.href.filter(|href| !one_line(href).is_empty()) {
            line.push_str(" -> ");
            line.push_str(&capped(href, HREF_CHARS));
        }
        line
    }
}

fn heading(offset: usize, shown: usize, total: usize) -> String {
    format!(
        "interactive elements {}-{} of {total}:",
        offset + 1,
        offset + shown
    )
}

fn omitted(count: usize, next_offset: usize) -> String {
    format!(
        "… {count} more interactive elements not listed. \
         Call browser_use_get_state with offset {next_offset}."
    )
}

/// Lines, characters and bytes of text joined with newlines.
#[derive(Clone, Copy)]
struct Size {
    lines: usize,
    chars: usize,
    bytes: usize,
}

impl Size {
    fn line(text: &str) -> Self {
        Self {
            lines: 1,
            chars: text.chars().count() + 1,
            bytes: text.len() + 1,
        }
    }

    fn of(lines: &[String]) -> Self {
        lines.iter().fold(
            Self {
                lines: 0,
                chars: 0,
                bytes: 0,
            },
            |size, line| size + Self::line(line),
        )
    }

    fn fits(&self) -> bool {
        self.lines <= MAX_LINES && self.chars <= MAX_CHARS && self.bytes <= MAX_BYTES
    }
}

impl std::ops::Add for Size {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            lines: self.lines + other.lines,
            chars: self.chars + other.chars,
            bytes: self.bytes + other.bytes,
        }
    }
}

/// `text` with every run of whitespace and control characters as one space.
fn one_line(text: &str) -> String {
    text.split(|c: char| c.is_whitespace() || c.is_control())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// At most `max` characters, the last one `…` when something was cut.
fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(max.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

fn capped(text: &str, max: usize) -> String {
    cut(&one_line(text), max)
}

/// A JSON string literal of the one-line, capped text.
fn quoted(text: &str, max: usize) -> String {
    serde_json::to_string(&capped(text, max)).unwrap_or_default()
}

/// A field name as one word.
fn name(key: &str) -> String {
    capped(key, TAG_CHARS).replace(' ', "_")
}

/// A value the pinned release does not define: strings quoted, the rest as
/// compact JSON.
fn value_text(value: &Value, max: usize) -> String {
    match value {
        Value::String(text) => quoted(text, max),
        other => cut(&other.to_string(), max),
    }
}
