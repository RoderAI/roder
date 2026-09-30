//! Roder's own direct CDP tools, bound to one tab.
//!
//! A [`DirectSession`] attaches to one tab ([`DirectTab`]: a target of a
//! browser's DevTools endpoint, or a page websocket) and runs the tools on
//! it: `look` (the page's elements with refs and boxes, and its text),
//! `screenshot`, `click`, `hover`, `drag`, `type`, `key`, `scroll`, `select`,
//! `navigate` and `wait`, all with real DevTools input. Whoever owns the tab
//! supplies a [`DirectGuard`] with its rules (allowed origins, a gate before
//! irreversible actions, typed secrets to keep out of what is read, what an
//! access block looks like); the tools enforce them and stop when they say
//! so.
//!
//! [`direct_tools`] turns the set into model-facing tools named
//! `<prefix>_<tool>`, each call handed its tab by a [`DirectBinding`]; Jev
//! binds them to its session's tab as `jev_tab_*`. [`devtools`] holds what
//! every DevTools client here shares (the browser websocket lookup, message
//! decoding, the dialog rule), which Jev's own connection uses too. Roder
//! Desktop's integrated browser, the `chrome_*` tools' fallback when no
//! extension is connected, goes through the same client.

mod act;
mod capture;
mod cleanup;
mod client;
pub mod devtools;
mod guard;
mod keys;
mod look;
mod session;
mod target;
mod tools;

pub use client::{DirectDialog, DirectTab};
pub use guard::{DirectGuard, GateAction, GateQuery, OpenGuard, PageFacts, StopKind};
pub use session::{DirectSession, DirectStep, DirectStop, OpenedTab};
pub use tools::{
    DIRECT_TOOLS, DirectBinding, DirectLease, direct_tool_specs, direct_tools, reads_only,
    tool_result,
};

pub(crate) use client::TabClient;
