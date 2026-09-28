//! The origins a run may visit, set by the operator.
//!
//! `JEV_ALLOWED_ORIGINS` is the real control: a per-call list can only narrow
//! it, since a prompt-injected caller could otherwise widen its own scope. An
//! origin is allowed when every scope in force allows it. The check runs
//! after each observation, so one page outside the scope has already loaded
//! when a run stops on it. Matching follows fastbrowse's safety.py (MIT): a
//! pattern is an exact origin, or one whose host starts with `*.` to cover
//! that host and every host under it, by whole labels only; the scheme and
//! port are never wildcarded, and a port the scheme implies is dropped.

use std::fmt;

use anyhow::{Context, bail, ensure};
use reqwest::Url;

const WILDCARD: &str = "*.";

/// One allowed origin: a scheme, a host (or a `*.` suffix) and a port.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Pattern {
    scheme: String,
    host: String,
    wildcard: bool,
    port: Option<u16>,
}

impl Pattern {
    fn parse(raw: &str) -> anyhow::Result<Self> {
        let raw = raw.trim().trim_end_matches('/');
        let (scheme, rest) = raw
            .split_once("://")
            .with_context(|| format!("allowed origin {raw:?} needs a scheme, such as https://"))?;
        let (wildcard, rest) = match rest.strip_prefix(WILDCARD) {
            Some(rest) => (true, rest),
            None => (false, rest),
        };
        let url = Url::parse(&format!("{scheme}://{rest}"))
            .ok()
            .filter(|url| matches!(url.scheme(), "http" | "https"))
            .with_context(|| format!("allowed origin {raw:?} is not an http(s) origin"))?;
        ensure!(
            url.path() == "/" && url.query().is_none() && url.fragment().is_none(),
            "allowed origin {raw:?} has a path; give only scheme, host and port"
        );
        ensure!(
            url.username().is_empty() && url.password().is_none(),
            "allowed origin {raw:?} carries credentials"
        );
        let host = url
            .host_str()
            .filter(|host| !host.is_empty())
            .with_context(|| format!("allowed origin {raw:?} has no host"))?;
        Ok(Self {
            scheme: url.scheme().into(),
            host: host.to_ascii_lowercase(),
            wildcard,
            port: url.port(),
        })
    }

    fn allows(&self, origin: &Origin) -> bool {
        if (self.scheme.as_str(), self.port) != (origin.scheme.as_str(), origin.port) {
            return false;
        }
        origin.host == self.host
            || (self.wildcard
                && origin
                    .host
                    .strip_suffix(&self.host)
                    .is_some_and(|head| head.ends_with('.')))
    }
}

impl fmt::Display for Pattern {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let star = if self.wildcard { WILDCARD } else { "" };
        write!(formatter, "{}://{star}{}", self.scheme, self.host)?;
        if let Some(port) = self.port {
            write!(formatter, ":{port}")?;
        }
        Ok(())
    }
}

/// A URL's web origin, with an implied port dropped (`Url` does that).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Origin {
    scheme: String,
    host: String,
    port: Option<u16>,
}

impl Origin {
    fn of(url: &str) -> Option<Self> {
        let url = Url::parse(url).ok()?;
        Some(Self {
            scheme: url.scheme().into(),
            host: url.host_str()?.to_ascii_lowercase(),
            port: url.port(),
        })
    }
}

/// Every scope in force: the operator's, then a call's.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JevOriginScope {
    scopes: Vec<Vec<Pattern>>,
}

impl JevOriginScope {
    /// No restriction.
    pub fn any() -> Self {
        Self::default()
    }

    /// Narrow to a list of origins: an origin must now also match one of
    /// these. An empty list is refused, since it would allow nothing.
    pub fn narrow<S: AsRef<str>>(mut self, origins: &[S]) -> anyhow::Result<Self> {
        let patterns = origins
            .iter()
            .map(|origin| Pattern::parse(origin.as_ref()))
            .collect::<anyhow::Result<Vec<_>>>()?;
        if patterns.is_empty() {
            bail!("an allowed-origins list must name at least one origin");
        }
        self.scopes.push(patterns);
        Ok(self)
    }

    /// Narrow by a comma- or space-separated list, as `JEV_ALLOWED_ORIGINS`
    /// is written.
    pub fn narrow_by_list(self, list: &str) -> anyhow::Result<Self> {
        let origins = list
            .split([',', ' ', '\n', '\t'])
            .filter(|origin| !origin.trim().is_empty())
            .collect::<Vec<_>>();
        self.narrow(&origins)
    }

    pub fn is_restricted(&self) -> bool {
        !self.scopes.is_empty()
    }

    /// Whether `url` may be visited. A blank page (a tab before it loads) is
    /// in every scope; any other URL without an http(s) origin is in none
    /// once a scope is set.
    pub fn allows(&self, url: &str) -> bool {
        if self.scopes.is_empty() || url == "about:blank" {
            return true;
        }
        let Some(origin) = Origin::of(url).filter(|origin| {
            matches!(origin.scheme.as_str(), "http" | "https") && !origin.host.is_empty()
        }) else {
            return false;
        };
        self.scopes
            .iter()
            .all(|scope| scope.iter().any(|pattern| pattern.allows(&origin)))
    }

    /// Why a run stopped on `url`.
    pub(crate) fn outside(&self, url: &str) -> String {
        let place = Origin::of(url).map_or_else(
            || url.chars().take(80).collect(),
            |origin| {
                let port = origin
                    .port
                    .map(|port| format!(":{port}"))
                    .unwrap_or_default();
                format!("{}://{}{port}", origin.scheme, origin.host)
            },
        );
        format!("The page went outside the allowed origins: {place} is not in {self}")
    }
}

impl fmt::Display for JevOriginScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.scopes.is_empty() {
            return write!(formatter, "any origin");
        }
        let scopes = self
            .scopes
            .iter()
            .map(|scope| {
                scope
                    .iter()
                    .map(Pattern::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .collect::<Vec<_>>();
        write!(formatter, "{}", scopes.join(" and within "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(origins: &[&str]) -> JevOriginScope {
        JevOriginScope::any().narrow(origins).unwrap()
    }

    #[test]
    fn an_exact_origin_matches_scheme_host_and_port() {
        let exact = scope(&["https://shop.example.com"]);
        assert!(exact.allows("https://shop.example.com/cart?x=1"));
        assert!(exact.allows("https://SHOP.example.com:443/"));
        assert!(!exact.allows("http://shop.example.com/"));
        assert!(!exact.allows("https://shop.example.com:8443/"));
        assert!(!exact.allows("https://example.com/"));
        assert!(!exact.allows("https://evil.test/?shop.example.com"));
        let ported = scope(&["http://127.0.0.1:8080"]);
        assert!(ported.allows("http://127.0.0.1:8080/pages/a.html"));
        assert!(!ported.allows("http://127.0.0.1:8081/"));
        assert!(!ported.allows("http://localhost:8080/"));
    }

    #[test]
    fn a_wildcard_covers_whole_labels_only() {
        let wide = scope(&["https://*.example.com"]);
        assert!(wide.allows("https://example.com/"));
        assert!(wide.allows("https://a.b.example.com/"));
        assert!(!wide.allows("https://badexample.com/"));
        assert!(!wide.allows("https://example.com.evil.test/"));
        assert!(!wide.allows("http://www.example.com/"));
    }

    #[test]
    fn a_call_can_only_narrow_the_operators_scope() {
        let operator = scope(&["https://*.example.com"]);
        let narrowed = operator
            .clone()
            .narrow(&["https://docs.example.com", "https://evil.test"])
            .unwrap();
        assert!(narrowed.allows("https://docs.example.com/a"));
        // Named by the call, but outside the operator's scope.
        assert!(!narrowed.allows("https://evil.test/"));
        assert!(!narrowed.allows("https://shop.example.com/"));
        assert_eq!(
            narrowed.to_string(),
            "https://*.example.com and within https://docs.example.com, https://evil.test"
        );
    }

    #[test]
    fn non_web_urls_are_outside_any_scope_but_a_blank_tab() {
        let restricted = scope(&["https://example.com"]);
        assert!(restricted.allows("about:blank"));
        for url in [
            "chrome-error://chromewebdata/",
            "data:text/html,hi",
            "file:///etc/hosts",
        ] {
            assert!(!restricted.allows(url), "{url}");
        }
        assert!(JevOriginScope::any().allows("chrome-error://chromewebdata/"));
    }

    #[test]
    fn malformed_origins_are_refused() {
        for bad in [
            "example.com",
            "ftp://example.com",
            "https://example.com/path",
            "https://user@example.com",
            "https://",
        ] {
            assert!(JevOriginScope::any().narrow(&[bad]).is_err(), "{bad}");
        }
        assert!(JevOriginScope::any().narrow::<&str>(&[]).is_err());
        let listed = JevOriginScope::any()
            .narrow_by_list(" https://a.test, https://b.test\nhttp://c.test:81 ")
            .unwrap();
        assert!(listed.allows("http://c.test:81/"));
        assert_eq!(
            listed.outside("https://d.test/x"),
            "The page went outside the allowed origins: https://d.test is not in \
             https://a.test, https://b.test, http://c.test:81"
        );
    }
}
