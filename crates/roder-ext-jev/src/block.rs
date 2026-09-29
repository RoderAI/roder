//! Telling a page that refused automated access from one Jev cannot act on.
//!
//! A site that blocks automation answers with a 401, 403 or 429, a challenge
//! page or an "unusual traffic" wall, and Jev used to report it as a page
//! with no controls, which sent callers hunting for canvas widgets. Here the
//! main document's HTTP status, its title, its text and its address are read
//! for the marks such pages carry. Detection and honest reporting only: Jev
//! never tries to get around a block.

use reqwest::Url;

/// What kind of refusal a page is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockKind {
    Forbidden,
    RateLimited,
    Challenge,
}

/// A page that refused automated access, with the evidence for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AccessBlock {
    pub(crate) kind: BlockKind,
    pub(crate) evidence: String,
}

impl AccessBlock {
    /// Why the run stopped, as the result says it.
    pub(crate) fn reason(&self) -> String {
        let what = match self.kind {
            BlockKind::Forbidden => "The site refused automated access",
            BlockKind::RateLimited => "The site rate-limited automated access",
            BlockKind::Challenge => "The site showed a check for automated access",
        };
        format!("{what} ({})", self.evidence)
    }
}

/// Phrases a refusal page shows, lower-cased, with the kind each implies.
const MARKERS: [(&str, BlockKind); 7] = [
    ("access denied", BlockKind::Forbidden),
    ("unusual traffic", BlockKind::RateLimited),
    ("too many requests", BlockKind::RateLimited),
    ("verify you are human", BlockKind::Challenge),
    ("are you a robot", BlockKind::Challenge),
    ("captcha", BlockKind::Challenge),
    ("checking your browser", BlockKind::Challenge),
];

/// Only the start of the text is read: a refusal page says it at once, and
/// an article that mentions a captcha further down is not one.
const TEXT_CHARS: usize = 600;

/// Whether the page at `url` refused automated access. `controls` is how
/// many elements it offers to act on. A marker alone is not enough on a page
/// that loaded (status 2xx or unknown) and offers controls: it must be in the
/// title, with nothing to act on. A 401, 403 or 429 with a marker or with
/// nothing to act on is one, and so is a `cdn-cgi/challenge` address. A
/// `/sorry/` address, the path search engines put their traffic check
/// under, counts only with more to show for it: a refusal status, a marker
/// in the page's opening text, or nothing to act on. Any site may have a
/// `/sorry/out-of-stock` page.
pub(crate) fn classify(
    status: Option<u16>,
    url: &str,
    title: &str,
    text: &str,
    controls: usize,
) -> Option<AccessBlock> {
    let path = Url::parse(url)
        .map(|url| url.path().to_ascii_lowercase())
        .unwrap_or_default();
    let title_lower = title.to_lowercase();
    let head = text
        .chars()
        .take(TEXT_CHARS)
        .collect::<String>()
        .to_lowercase();
    let in_title = MARKERS
        .iter()
        .find(|(marker, _)| title_lower.contains(marker));
    let in_text = MARKERS.iter().find(|(marker, _)| head.contains(marker));
    let refusing = matches!(status, Some(401 | 403 | 429));
    let sorry = path.starts_with("/sorry/")
        && (refusing || in_title.or(in_text).is_some() || controls == 0);
    if sorry || path.contains("/cdn-cgi/challenge") {
        return Some(AccessBlock {
            kind: BlockKind::Challenge,
            evidence: with_status(status, format!("challenge page {path}")),
        });
    }
    let quoted = |marker: &str| match in_title {
        Some(_) => format!("{:?}", title.trim()),
        None => format!("{marker:?}"),
    };
    match status {
        Some(code @ (401 | 403 | 429)) => {
            let found = in_title.or(in_text);
            if found.is_none() && controls > 0 {
                return None;
            }
            let kind = match (code, found) {
                (_, Some((_, BlockKind::Challenge))) => BlockKind::Challenge,
                (429, _) | (_, Some((_, BlockKind::RateLimited))) => BlockKind::RateLimited,
                _ => BlockKind::Forbidden,
            };
            let evidence = match found {
                Some((marker, _)) => format!("HTTP {code}, {}", quoted(marker)),
                None => format!("HTTP {code}, nothing to act on"),
            };
            Some(AccessBlock { kind, evidence })
        }
        _ if controls == 0 => in_title.map(|(marker, kind)| AccessBlock {
            kind: *kind,
            evidence: with_status(status, quoted(marker)),
        }),
        _ => None,
    }
}

fn with_status(status: Option<u16>, evidence: String) -> String {
    match status {
        Some(code) => format!("HTTP {code}, {evidence}"),
        None => evidence,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_a_403_access_denied_page() {
        let block = classify(
            Some(403),
            "https://www.shop.test/s/restaurants",
            "Access Denied",
            "You don't have permission to access this server.",
            0,
        )
        .unwrap();
        assert_eq!(block.kind, BlockKind::Forbidden);
        assert_eq!(
            block.reason(),
            "The site refused automated access (HTTP 403, \"Access Denied\")"
        );
        // A 403 with nothing to act on needs no marker.
        let bare = classify(Some(403), "https://a.test/", "", "Forbidden", 0).unwrap();
        assert_eq!(bare.evidence, "HTTP 403, nothing to act on");
    }

    #[test]
    fn classify_a_429_as_rate_limited() {
        let block = classify(Some(429), "https://a.test/", "", "Slow down", 0).unwrap();
        assert_eq!(block.kind, BlockKind::RateLimited);
        assert!(
            block.reason().contains("rate-limited"),
            "{}",
            block.reason()
        );
    }

    #[test]
    fn classify_a_sorry_or_challenge_address() {
        let sorry = classify(
            Some(200),
            "https://www.search.test/sorry/index?continue=x",
            "",
            "Our systems have detected unusual traffic",
            3,
        )
        .unwrap();
        assert_eq!(sorry.kind, BlockKind::Challenge);
        assert!(sorry.evidence.contains("/sorry/"), "{}", sorry.evidence);
        assert!(
            classify(
                None,
                "https://a.test/cdn-cgi/challenge-platform/x",
                "",
                "",
                5
            )
            .is_some()
        );
    }

    /// Only a `/sorry/` page that shows more of a refusal is one: an
    /// ordinary page under that path is not.
    #[test]
    fn classify_a_sorry_address_only_with_corroboration() {
        assert_eq!(
            classify(
                Some(200),
                "https://shop.test/sorry/out-of-stock",
                "Sorry, out of stock",
                "This item is out of stock. Browse similar items.",
                12
            ),
            None
        );
        for (status, text, controls) in [
            (Some(429), "Slow down", 3),
            (Some(200), "", 0),
            (None, "Our systems have detected unusual traffic", 2),
        ] {
            let block = classify(status, "https://s.test/sorry/index", "", text, controls);
            assert_eq!(block.map(|block| block.kind), Some(BlockKind::Challenge));
        }
    }

    #[test]
    fn classify_a_captcha_title_with_nothing_to_act_on() {
        let block = classify(None, "https://a.test/", "Verify you are human", "", 0).unwrap();
        assert_eq!(block.kind, BlockKind::Challenge);
        assert_eq!(block.evidence, "\"Verify you are human\"");
    }

    #[test]
    fn classify_no_false_positive_on_a_page_that_mentions_a_captcha() {
        let article = "How CAPTCHA works\nA captcha asks you to verify you are human.";
        assert_eq!(
            classify(
                Some(200),
                "https://blog.test/captcha",
                "How CAPTCHA works",
                article,
                40
            ),
            None
        );
        assert_eq!(
            classify(None, "https://blog.test/", "Blog", article, 0),
            None
        );
        // A 403 page that still offers controls and says nothing of a block
        // is an ordinary error page.
        assert_eq!(
            classify(Some(403), "https://a.test/", "Not allowed", "Go home", 2),
            None
        );
        assert_eq!(
            classify(Some(404), "https://a.test/", "Not found", "", 0),
            None
        );
    }
}
