//! What the owner of a tab enforces on the direct tools.
//!
//! The tools themselves stop before nothing: a component that hands them a
//! tab (Jev's session) knows its operator's allowed origins, its
//! irreversible-action gate, the secrets typed in that tab and what an
//! access block looks like, and says so through a [`DirectGuard`]. Every
//! method has a harmless default, so Roder Desktop's integrated browser
//! runs with [`OpenGuard`].

use serde::Serialize;

/// What an action about to be dispatched would press.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GateAction {
    /// A click (or a drag's release) on a control.
    Click,
    /// Enter in a field, or typing with `submit`.
    Enter,
}

/// The control an action would press, as the page names it (page text,
/// untrusted).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GateQuery {
    pub action: GateAction,
    /// The control's accessible name.
    pub label: String,
    /// Its ARIA role, or tag when it has none.
    pub role: Option<String>,
    /// Whether it is a form's submit button.
    pub submit: bool,
    /// The labels of the submit controls of the form it sits in.
    pub form_labels: Vec<String>,
    /// That form holds a filled password or one-time-code field.
    pub secret_form: bool,
    /// It sits in a region that talks about cookies or consent (a banner).
    pub consent: bool,
    /// It lands in a frame of another site, which the tools cannot see
    /// into: what it presses there is unknown (`label` names the frame).
    pub frame: bool,
}

/// A page as the guard checks it after an action: where it is, what it
/// says first, and how much it offers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct PageFacts {
    pub url: String,
    pub title: String,
    /// The first 600 characters of its text.
    pub text: String,
    /// The main document's HTTP status, when the browser reports it.
    pub http_status: Option<u16>,
    /// How many elements the page offers to act on.
    pub controls: usize,
}

/// Why a direct action stopped the run it belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StopKind {
    /// The action may not be undone and was not authorized; nothing was
    /// dispatched.
    NeedsConfirmation,
    /// The page is outside the origins the owner allows.
    OutsideScope,
    /// The site refused automated access.
    AccessDenied,
}

/// The owner's rules for its tab.
pub trait DirectGuard: Send + Sync {
    /// Why `url` may not be visited, when it may not.
    fn outside(&self, _url: &str) -> Option<String> {
        None
    }

    /// Why the tools never press this control, when they do not (a consent
    /// banner the owner leaves alone). The call fails with the reason and
    /// the run goes on; nothing is dispatched.
    fn refuses(&self, _query: &GateQuery) -> Option<String> {
        None
    }

    /// Why the pressed control needs the user's confirmation first, when it
    /// does. Returning a reason stops the action before it is dispatched.
    fn confirm(&self, _query: &GateQuery) -> Option<String> {
        None
    }

    /// Page-supplied text with the owner's typed secrets taken out.
    /// Retain secrets typed by a batched computer call before its screenshot.
    fn remember_secret(&self, _value: &str) {}

    fn scrub(&self, text: &str) -> String {
        text.to_string()
    }

    /// Why the page refused automated access, when it did. The tools never
    /// try to get around a refusal; they stop.
    fn refused(&self, _facts: &PageFacts) -> Option<String> {
        None
    }
}

/// No limits beyond the tools' own, for a browser Roder does not share with
/// a component that keeps its own rules.
pub struct OpenGuard;

impl DirectGuard for OpenGuard {}
