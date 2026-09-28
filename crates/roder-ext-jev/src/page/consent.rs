//! Refusing cookie and consent banners: on by default.
//!
//! Two layers, never both acting on one document (`refuse.js` sets the
//! order). DuckDuckGo's autoconsent (MPL-2.0, vendored unmodified in
//! `assets/autoconsent/`, see its `SOURCE.txt`) is injected into every new
//! document of every tab Jev owns before it loads, as fastbrowse (MIT) does,
//! and refuses consent on the platforms it knows, in its own opt-out mode.
//! `autoconsent_setup.js`, Jev's own, runs after it and holds its heuristics
//! to refusing (it initialises them to also press a lone "Accept" or "OK").
//! `consent.js`, Jev's own, is the conservative fallback for banners it does
//! not recognise: once per document, after the page settles and before the
//! first observation, it clicks a banner's refusal button only when the
//! banner is unambiguous, and never an accept or settings button. Neither
//! needs a model call. `consent.js` reaches neither frames nor closed shadow
//! roots, and a banner drawn after the page first goes quiet is left to
//! autoconsent.

use serde_json::json;

use super::Page;

/// DuckDuckGo's autoconsent, exactly as published (see `SOURCE.txt`).
pub(crate) const AUTOCONSENT_JS: &str =
    include_str!("../assets/autoconsent/autoconsent.standalone.js");
/// Jev's setting for it: heuristics that only ever refuse.
pub(crate) const AUTOCONSENT_SETUP_JS: &str = include_str!("../assets/autoconsent_setup.js");
pub(crate) const CONSENT_JS: &str = include_str!("../assets/consent.js");
pub(crate) const REFUSE_JS: &str = include_str!("../assets/refuse.js");

impl Page {
    /// Inject autoconsent into every document this page's tabs load from
    /// now on, the tabs Jev adopts later included. Set before `load`.
    pub(crate) fn inject_autoconsent(&mut self, on: bool) {
        self.autoconsent = on;
    }

    /// Put autoconsent on every new document of the current tab, and, for a
    /// tab adopted after it loaded (`loaded`), on the document it shows now
    /// too; the bundle skips a document it already runs in.
    pub(super) async fn install_autoconsent(&mut self, loaded: bool) -> anyhow::Result<()> {
        if !self.autoconsent {
            return Ok(());
        }
        // New-document scripts run in the order they were added.
        for source in [AUTOCONSENT_JS, AUTOCONSENT_SETUP_JS] {
            self.call(
                "Page.addScriptToEvaluateOnNewDocument",
                json!({"source": source}),
            )
            .await?;
        }
        if loaded {
            // Best effort: a document replaced meanwhile gets the new-document scripts.
            for source in [AUTOCONSENT_JS, AUTOCONSENT_SETUP_JS] {
                self.evaluate(source).await.ok();
            }
        }
        Ok(())
    }

    /// Settle any pending input, then let autoconsent finish or the fallback
    /// refuse this document's cookie banner; the refusal's effects settle
    /// like an action's. Returns what was clicked, as page text: the
    /// fallback's button label, or the platform autoconsent refused on.
    pub(crate) async fn refuse_cookie_banner(&mut self) -> anyhow::Result<Option<String>> {
        self.settle().await;
        let outcome = self
            .evaluate_async(&format!("({REFUSE_JS})({CONSENT_JS})"))
            .await?;
        let label = match (outcome["autoconsent"].as_str(), outcome["clicked"].as_str()) {
            (Some(platform), _) => format!("{platform} (autoconsent)"),
            (None, Some(label)) => label.to_string(),
            (None, None) => return Ok(None),
        };
        self.settle_page(None).await;
        Ok(Some(label))
    }
}

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};

    use super::*;

    /// The vendored bundle is exactly the published 16.40.0 file; any change
    /// to it is a supply-chain change (see `SOURCE.txt`).
    #[test]
    fn the_vendored_autoconsent_is_the_pinned_file() {
        let digest = Sha256::digest(AUTOCONSENT_JS.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(
            digest,
            "cc50f2114cce0bdaf4f7da703cae4467a84d609d234f3ccb69c5ec0f071aca97"
        );
        assert_eq!(AUTOCONSENT_JS.len(), 440_566);
        let source = include_str!("../assets/autoconsent/SOURCE.txt");
        assert!(source.contains(&digest), "SOURCE.txt names another hash");
        let licence = include_str!("../assets/autoconsent/LICENSE");
        assert!(licence.starts_with("Mozilla Public License Version 2.0"));
    }
}
