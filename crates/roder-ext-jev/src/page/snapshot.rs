//! The observation script, compiled once in each document and called by name.
//!
//! Sent whole with every read, `snapshot.js` and its parts cost a transfer
//! and a compile per call, and each call started again from unoptimized
//! code. Instead every new document installs it as it starts (registered
//! with the quiet clock, see `settle`), and a document older than that
//! registration installs it when a call first finds it missing: the call
//! throws [`UNINSTALLED`] before doing anything, so it is made again once
//! installed. The global is keyed by a digest of the scripts and a random
//! per-process seed, so another build never calls this one's function and a
//! page cannot plant one under the name in advance; it is defined read-only.
//! The mechanism follows fastbrowse's `SnapshotScript` (MIT).

use std::hash::{BuildHasher, Hash, Hasher};
use std::sync::LazyLock;

use serde_json::Value;

use super::settle::TRACK_JS;
use super::{CONTEXT_JS, Page, SNAPSHOT_JS};

/// What a call on the installed scripts throws in a document without them.
const UNINSTALLED: &str = "jev: the snapshot is not installed in this document";

/// DevTools' `getEventListeners` exists only in the evaluated expression's
/// own scope, so each call hands it to the installed function, whose
/// parameter of that name `snapshot.js` reads as before.
const LISTENERS: &str = "typeof getEventListeners==='function'?getEventListeners:undefined";

/// A call on the installed scripts threw [`UNINSTALLED`].
#[derive(Debug)]
pub(super) struct Uninstalled;

impl std::fmt::Display for Uninstalled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(UNINSTALLED)
    }
}

impl std::error::Error for Uninstalled {}

/// Whether a `Runtime.evaluate` exception is the missing-install sentinel.
pub(super) fn is_uninstalled(details: &Value) -> bool {
    details.to_string().contains(UNINSTALLED)
}

/// What an installed call returns.
#[derive(Clone, Copy)]
pub(super) enum Read {
    /// The observation: the quiet clock, the snapshot, then `context.js`.
    Observe,
    /// Only the snapshot's freshness marker.
    Marker,
}

impl Read {
    fn method(self) -> &'static str {
        match self {
            Self::Observe => "observe",
            Self::Marker => "marker",
        }
    }
}

struct Scripts {
    /// Installs the scripts in this document, if it has none.
    install: String,
    /// [`Self::install`], for every new document's top frame.
    new_document: String,
    /// The global the scripts are installed under.
    key: String,
    /// An expression for the scripts themselves, for a page that refuses
    /// the install.
    inline: String,
}

static SCRIPTS: LazyLock<Scripts> = LazyLock::new(|| {
    let mut seed = std::collections::hash_map::RandomState::new().build_hasher();
    (TRACK_JS, CONTEXT_JS, SNAPSHOT_JS).hash(&mut seed);
    Scripts::new(&format!("__jev{:016x}", seed.finish()))
});

impl Scripts {
    fn new(key: &str) -> Self {
        let inline = format!(
            "(() => {{ const snapshot=getEventListeners=>{SNAPSHOT_JS};\n\
             return Object.freeze({{snapshot,\n\
             marker:getEventListeners=>snapshot(getEventListeners)?.marker ?? null,\n\
             observe:getEventListeners=>{{\n{TRACK_JS}\n\
             return ({CONTEXT_JS})(snapshot(getEventListeners));\n}}}}); }})()"
        );
        // A page that froze its global object refuses the install; reads
        // then send the scripts whole (see `Page::read`).
        let install = format!(
            "(() => {{ try {{ if (!Object.hasOwn(globalThis,'{key}')) \
             Object.defineProperty(globalThis,'{key}',{{value:{inline}}}); }} catch {{}} }})()"
        );
        Self {
            new_document: format!("if (window===window.top) {install}"),
            install,
            key: key.to_string(),
            inline,
        }
    }

    /// The short expression a read sends once the scripts are installed.
    fn call(&self, read: Read) -> String {
        let key = &self.key;
        let missing = serde_json::to_string(UNINSTALLED).expect("a string serializes");
        format!(
            "(globalThis['{key}'] ?? (() => {{ throw new Error({missing}); }})()).{}({LISTENERS})",
            read.method()
        )
    }

    fn inline_call(&self, read: Read) -> String {
        format!("({}).{}({LISTENERS})", self.inline, read.method())
    }
}

impl Page {
    /// Install the observation scripts on every document this tab loads
    /// from now on.
    pub(super) async fn install_snapshot(&mut self) -> anyhow::Result<()> {
        self.call(
            "Page.addScriptToEvaluateOnNewDocument",
            serde_json::json!({"source": SCRIPTS.new_document}),
        )
        .await
        .map(|_| ())
    }

    /// Run one installed read, installing the scripts first in a document
    /// that has none, and sending them whole to one that refuses them.
    pub(super) async fn read(&mut self, read: Read) -> anyhow::Result<Value> {
        let call = SCRIPTS.call(read);
        match self.evaluate_snapshot(&call).await {
            Err(error) if error.is::<Uninstalled>() => {}
            result => return result,
        }
        self.evaluate(&SCRIPTS.install).await?;
        match self.evaluate_snapshot(&call).await {
            Err(error) if error.is::<Uninstalled>() => {
                self.evaluate_snapshot(&SCRIPTS.inline_call(read)).await
            }
            result => result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_read_sends_a_call_not_the_scripts() {
        let call = SCRIPTS.call(Read::Observe);
        assert!(call.len() < 300, "{call}");
        assert!(call.contains(&SCRIPTS.key));
        assert!(call.ends_with(&format!(".observe({LISTENERS})")));
        assert!(SCRIPTS.install.contains(SNAPSHOT_JS));
        assert!(SCRIPTS.new_document.ends_with(&SCRIPTS.install));
        assert!(SCRIPTS.inline_call(Read::Marker).contains(SNAPSHOT_JS));
        // Unguessable: the key carries 64 bits of the process's random seed.
        assert_eq!(SCRIPTS.key.len(), "__jev".len() + 16);
    }

    #[test]
    fn the_sentinel_is_read_off_the_exception() {
        let details = serde_json::json!({"exception": {"description": format!("Error: {UNINSTALLED}\n    at <anonymous>:1:2")}});
        assert!(is_uninstalled(&details));
        assert!(!is_uninstalled(
            &serde_json::json!({"exception": {"description": "TypeError: x"}})
        ));
    }
}
