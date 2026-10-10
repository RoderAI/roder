//! The text a completion check compares: what a caller writes and what the
//! page shows are both reduced to one comparable form first, and what a form
//! field holds is not counted as the page's own text. Page text is folded
//! to lower case; a URL keeps the case of everything but its scheme and host,
//! because a server can serve another page at a path in another case.

/// Characters that take no room on screen, so a page can carry them anywhere
/// in a word and a caller cannot see them: the soft hyphen, the combining
/// grapheme joiner, the Arabic letter mark, the Mongolian vowel separator,
/// the zero-width space, joiners and directional marks, the bidirectional
/// embedding and isolate controls, the invisible operators and the byte-order
/// mark.
fn invisible(c: char) -> bool {
    matches!(
        c,
        '\u{ad}'
            | '\u{34f}'
            | '\u{61c}'
            | '\u{180e}'
            | '\u{200b}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206f}'
            | '\u{feff}'
    )
}

/// The Greek small letter final sigma and the small sigma it stands for.
const FINAL_SIGMA: char = '\u{3c2}';
const SIGMA: char = '\u{3c3}';

/// The comparable form of `text`: invisible characters dropped, every run of
/// whitespace (line breaks between nodes, tabs, no-break and other
/// typographic spaces) one space, nothing at either end, and lower case,
/// with the Greek final sigma written as the ordinary one (lower-casing a
/// character alone turns a capital sigma into the ordinary one only, so a
/// word in capitals would not match the same word in lower case). Applied
/// to the page and to the caller's predicate alike.
pub(crate) fn fold(text: &str) -> String {
    squash(text, true)
}

/// The comparable form of a URL, or of a string meant to be found in one:
/// like [`fold`] but with the case kept, since the path, the query and the
/// fragment are case-sensitive (`/Orders/ABC` and `/orders/abc` can be two
/// pages), and for a full URL (`scheme://...`) the scheme and the host in
/// lower case, which are not. A string that does not start with a scheme and
/// `://`, such as `/orders/abc` or `shop.test/orders`, is compared as it is
/// written. Applied to the page's URL and to the caller's predicate alike.
pub(crate) fn fold_url(url: &str) -> String {
    let squashed = squash(url, false);
    let Some(scheme_end) = squashed
        .find("://")
        .filter(|&end| is_scheme(&squashed[..end]))
    else {
        return squashed;
    };
    let authority = scheme_end + "://".len();
    let authority_end = squashed[authority..]
        .find(['/', '?', '#'])
        .map_or(squashed.len(), |end| authority + end);
    // What stands before an `@` is a name and a password, which keep their case.
    let host = squashed[authority..authority_end]
        .rfind('@')
        .map_or(authority, |at| authority + at + 1);
    format!(
        "{}://{}{}{}",
        squashed[..scheme_end].to_ascii_lowercase(),
        &squashed[authority..host],
        squashed[host..authority_end].to_lowercase(),
        &squashed[authority_end..]
    )
}

/// Whether `text` is a URL scheme: a letter, then letters, digits, `+`, `-`
/// and `.`.
fn is_scheme(text: &str) -> bool {
    let mut chars = text.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// `text` with invisible characters dropped and every run of whitespace one
/// space, nothing at either end; in lower case when `lower` is set.
fn squash(text: &str, lower: bool) -> String {
    let mut folded = String::with_capacity(text.len());
    let mut gap = false;
    for c in text.chars().filter(|&c| !invisible(c)) {
        if c.is_whitespace() {
            // Not at the start, and only written when something follows.
            gap = !folded.is_empty();
            continue;
        }
        if gap {
            folded.push(' ');
            gap = false;
        }
        if lower {
            folded.extend(
                c.to_lowercase()
                    .map(|lower| if lower == FINAL_SIGMA { SIGMA } else { lower }),
            );
        } else {
            folded.push(c);
        }
    }
    folded
}

/// `text` without the lines that only echo a value a form field holds. The
/// page text lists such a value on a line of its own (see `text.js`), and
/// `typed` names each one it listed. One line is taken out for each value,
/// so the same words shown by the page itself, as well as in a field, still
/// count.
pub(crate) fn without_echoes(text: &str, typed: &[String]) -> String {
    let mut kept = text.to_owned();
    for value in typed.iter().filter(|value| !value.is_empty()) {
        let Some(start) = line_of(&kept, value) else {
            continue;
        };
        let end = start + value.len();
        // The line break that follows goes with it, else the one before.
        let (from, to) = match (kept[end..].starts_with('\n'), start) {
            (true, _) => (start, end + 1),
            (false, 0) => (start, end),
            (false, _) => (start - 1, end),
        };
        kept.replace_range(from..to, "");
    }
    kept
}

/// Where `value` stands as whole lines of `text`, the first time.
fn line_of(text: &str, value: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut from = 0;
    while let Some(found) = text[from..].find(value) {
        let start = from + found;
        let end = start + value.len();
        if (start == 0 || bytes[start - 1] == b'\n') && (end == bytes.len() || bytes[end] == b'\n')
        {
            return Some(start);
        }
        from = start + text[start..].chars().next().map_or(1, char::len_utf8);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn fold_reduces_case_and_every_kind_of_whitespace_to_one_form() {
        assert_eq!(fold("Count:\n1"), "count: 1");
        assert_eq!(fold("  COUNT: \t\r\n 1  "), "count: 1");
        assert_eq!(
            fold("Order\u{a0}\u{a0}No.\u{2007}7\u{202f}\u{3000}done"),
            "order no. 7 done"
        );
        assert_eq!(fold("Straße ÉCOLE"), "straße école");
        assert_eq!(fold(""), "");
        assert_eq!(fold(" \n\t\u{a0}"), "");
    }

    #[test]
    fn fold_makes_a_final_sigma_the_ordinary_one() {
        // Lower-casing a character alone turns capital sigma into the
        // ordinary one, never the final one, so a page's word-final sigma
        // would stay apart from the same word in capitals.
        assert_eq!(fold("ΟΣ"), fold("ος"));
        assert_eq!(fold("ΚΟΣΜΟΣ"), "κοσμοσ");
        assert_eq!(fold("κοσμος"), "κοσμοσ");
        assert_eq!(fold("Κοσμος σοφος"), fold("ΚΟΣΜΟΣ ΣΟΦΟΣ"));
        // Nothing else about the word changes.
        assert_eq!(fold("ΣΊΣΥΦΟΣ"), "σίσυφοσ");
        assert_eq!(fold("ς"), "σ");
    }

    #[test]
    fn fold_drops_zero_width_and_invisible_characters_without_making_a_gap() {
        assert_eq!(fold("Con\u{200b}firmed"), "confirmed");
        assert_eq!(
            fold("Con\u{200c}fir\u{200d}med\u{2060}!\u{feff}"),
            "confirmed!"
        );
        assert_eq!(fold("sup\u{ad}er \u{200e}man\u{200f}"), "super man");
        // A zero-width character beside whitespace joins neither side.
        assert_eq!(fold("a \u{200b} b"), "a b");
        assert_eq!(fold("\u{feff}\n\u{200b}x"), "x");
        // A caller's predicate and the page fold to the same form.
        assert_eq!(
            fold("ORDER\u{a0}\u{a0}Con\u{200b}firmed"),
            fold("order confirmed")
        );
    }

    #[test]
    fn fold_keeps_text_that_is_not_a_space_or_invisible() {
        assert_eq!(fold("a-b_c/d?e=1&f"), "a-b_c/d?e=1&f");
        assert_eq!(fold("日本語 テキスト"), "日本語 テキスト");
        assert_eq!(fold("🙂\u{200d}🙂"), "🙂🙂");
    }

    #[test]
    fn fold_url_keeps_the_case_of_everything_after_the_host() {
        assert_eq!(
            fold_url("https://shop.test/Orders/ABC?Q=Boots&p=2#Top"),
            "https://shop.test/Orders/ABC?Q=Boots&p=2#Top"
        );
        assert_eq!(
            fold_url("HTTPS://Shop.Test:8443/Orders/ABC?Q=Boots#Top"),
            "https://shop.test:8443/Orders/ABC?Q=Boots#Top"
        );
        // The host ends at the query or the fragment as well as the path.
        assert_eq!(fold_url("http://Shop.Test?Q=1"), "http://shop.test?Q=1");
        assert_eq!(fold_url("http://Shop.Test#Top"), "http://shop.test#Top");
        assert_eq!(fold_url("HTTP://Shop.Test"), "http://shop.test");
        // A name and a password are not part of the host.
        assert_eq!(
            fold_url("https://Ann:Secret@Shop.Test/Orders"),
            "https://Ann:Secret@shop.test/Orders"
        );
        assert_eq!(fold_url("ftp://U@@Host/P"), "ftp://U@@host/P");
        assert_eq!(
            fold_url("chrome-extension://ABC/Page.HTML"),
            "chrome-extension://abc/Page.HTML"
        );
    }

    #[test]
    fn fold_url_compares_a_string_that_is_not_a_full_url_as_written() {
        for as_written in [
            "/Orders/ABC",
            "Shop.Test/Orders/ABC",
            "?Q=Boots",
            "#Top",
            "//Shop.Test/Orders",
            "Orders/ABC",
            "",
            // A URL inside a query is the query's text, not the address.
            "/Redirect?To=HTTPS://Shop.Test/Orders",
            "not a scheme://Host/Path",
            "1http://Host/Path",
            "://Host/Path",
            "日本語/Ñandú",
        ] {
            assert_eq!(fold_url(as_written), as_written);
        }
    }

    #[test]
    fn fold_url_drops_zero_width_characters_and_folds_whitespace() {
        assert_eq!(
            fold_url(" HTTPS://Shop.\u{200b}Test/Or\u{feff}ders/ABC\n"),
            "https://shop.test/Orders/ABC"
        );
        assert_eq!(fold_url("/A \t\u{a0}b"), "/A b");
        assert_eq!(fold_url("\u{200b}  "), "");
    }

    #[test]
    fn a_line_that_echoes_a_field_value_is_taken_out() {
        let text = "Order\nNote\norder confirmed\nSaving order\nFinish order";
        assert_eq!(
            without_echoes(text, &typed(&["order confirmed"])),
            "Order\nNote\nSaving order\nFinish order"
        );
        // First line, last line, only line.
        assert_eq!(without_echoes("a\nb\nc", &typed(&["a"])), "b\nc");
        assert_eq!(without_echoes("a\nb\nc", &typed(&["c"])), "a\nb");
        assert_eq!(without_echoes("a", &typed(&["a"])), "");
    }

    #[test]
    fn only_a_whole_line_is_an_echo() {
        // "order" is a word in a longer line, not a line of its own.
        let text = "Your order is ready\nan order";
        assert_eq!(without_echoes(text, &typed(&["order"])), text);
        assert_eq!(without_echoes(text, &typed(&["Your order"])), text);
        assert_eq!(without_echoes(text, &typed(&[])), text);
        assert_eq!(without_echoes(text, &typed(&[""])), text);
    }

    #[test]
    fn a_value_the_page_shows_too_still_counts_once() {
        // A field holds "Count: 1" and so does the page, under a heading.
        let text = "Count: 1\nTotal\nCount: 1";
        assert_eq!(
            without_echoes(text, &typed(&["Count: 1"])),
            "Total\nCount: 1"
        );
        assert_eq!(
            without_echoes(text, &typed(&["Count: 1", "Count: 1"])),
            "Total"
        );
    }

    #[test]
    fn a_many_line_value_is_taken_out_as_one() {
        let text = "Notes\nline one\nline two\nSave";
        assert_eq!(
            without_echoes(text, &typed(&["line one\nline two"])),
            "Notes\nSave"
        );
        // Half of it is not the whole value.
        assert_eq!(
            without_echoes(text, &typed(&["line two\nSave\nmore"])),
            text
        );
    }

    #[test]
    fn what_the_text_limit_left_of_a_value_is_taken_out_as_its_last_lines() {
        // The page text ends inside a field's three-line value; `text.js`
        // reports the part it kept, which is those last lines exactly.
        let text = "Notes\nline one\nline two\nline th";
        assert_eq!(
            without_echoes(text, &typed(&["line one\nline two\nline th"])),
            "Notes"
        );
        // The whole value is not what the page shows: nothing matches.
        assert_eq!(
            without_echoes(text, &typed(&["line one\nline two\nline three"])),
            text
        );
    }

    #[test]
    fn multibyte_text_is_cut_on_character_boundaries() {
        let text = "日本\nñandú\n日本";
        assert_eq!(without_echoes(text, &typed(&["ñandú"])), "日本\n日本");
        assert_eq!(without_echoes(text, &typed(&["本"])), text);
    }
}
