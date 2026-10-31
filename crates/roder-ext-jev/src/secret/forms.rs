//! The forms a typed secret is carried in, and the hosts of the addresses in
//! a text. The encodings follow fastbrowse's `Redactor` (MIT): a page, an
//! address or a log carries a value encoded as often as raw.

use reqwest::Url;

/// Every form of `value` worth looking for, longest first, with no
/// duplicates: the value; its whitespace collapsed as ECMAScript's `\s`
/// splits it (accessible names and `innerText` do this); each of those
/// percent-encoded as `encodeURIComponent` does and as a form submission
/// does (`%20` or `+` for a space either way); and each escaped inside a
/// JSON string, with and without `\uXXXX` for non-ASCII.
pub(super) fn forms(value: &str) -> Vec<String> {
    let collapsed = value
        .split(|c: char| (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let mut forms: Vec<String> = Vec::new();
    for base in [value, collapsed.as_str()] {
        if base.is_empty() {
            continue;
        }
        let component = percent(base, |b| b"-_.!~*'()".contains(&b));
        let form = percent(base, |b| b"*-._".contains(&b));
        for encoded in [
            base.to_string(),
            component.clone(),
            component.replace("%20", "+"),
            form.clone(),
            form.replace("%20", "+"),
            json_escaped(base, false),
            json_escaped(base, true),
        ] {
            if !forms.contains(&encoded) {
                forms.push(encoded);
            }
        }
    }
    // Longest first, so a form found inside a longer one is merged into it.
    forms.sort_by_key(|form| std::cmp::Reverse(form.len()));
    forms
}

/// `value` percent-encoded byte by byte, keeping ASCII letters, digits and
/// the bytes `keep` names, with upper-case hex as browsers write it.
fn percent(value: &str, keep: impl Fn(u8) -> bool) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || keep(byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// What `value` reads as between the quotes of a JSON string; with `ascii`,
/// every non-ASCII character as `\uXXXX` (Python's `ensure_ascii`).
fn json_escaped(value: &str, ascii: bool) -> String {
    let quoted = serde_json::to_string(value).expect("string serialization");
    let inner = &quoted[1..quoted.len() - 1];
    if !ascii {
        return inner.to_string();
    }
    let mut out = String::with_capacity(inner.len());
    for c in inner.chars() {
        if c.is_ascii() {
            out.push(c);
            continue;
        }
        let mut units = [0u16; 2];
        for unit in c.encode_utf16(&mut units) {
            out.push_str(&format!("\\u{unit:04x}"));
        }
    }
    out
}

/// The origin of `url`, for an address with one (`https://host:port`).
pub(super) fn origin_of(url: &str) -> Option<String> {
    let origin = Url::parse(url).ok()?.origin();
    origin.is_tuple().then(|| origin.ascii_serialization())
}

/// The host (and port) of every address in `text`, as a byte range with the
/// origin it belongs to. An address is `scheme://` and an authority running
/// to the first space, `/`, `?`, `#`, `\`, `<`, `>`, quote or backtick; a
/// sentence's full stop or a closing bracket after it is not part of it.
pub(super) fn hosts(text: &str) -> Vec<(usize, usize, String)> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if !bytes[at].is_ascii_alphabetic() {
            at += 1;
            continue;
        }
        let mut end = at + 1;
        while end < bytes.len()
            && (bytes[end].is_ascii_alphanumeric() || matches!(bytes[end], b'+' | b'.' | b'-'))
        {
            end += 1;
        }
        if !text[end..].starts_with("://") {
            at = end;
            continue;
        }
        let start = end + 3;
        let length = text[start..]
            .find(|c: char| {
                c.is_whitespace()
                    || matches!(c, '/' | '?' | '#' | '\\' | '<' | '>' | '"' | '\'' | '`')
            })
            .unwrap_or(text.len() - start);
        let authority = text[start..start + length]
            .trim_end_matches(['.', ',', ';', ':', '!', '?', ')', '}', '\'', '"']);
        if let Some(origin) = origin_of(&format!("{}://{authority}", &text[at..end])) {
            let host_end = start + authority.len();
            let host = authority
                .rsplit_once('@')
                .map_or(authority, |(_, host)| host);
            found.push((host_end - host.len(), host_end, origin));
        }
        at = start + length;
    }
    found
}
