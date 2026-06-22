//! Lightweight client/transcript layer shared by `roder-app-server` and its
//! consumers (`roder-tui`, `roder-cli`, `roder-app-server-node`).
//!
//! This crate holds only the `AppClient` trait, its event/notification
//! receivers, and the transcript recorder — none of which depend on the heavy
//! `AppServer` implementation in `roder-app-server::server`. Splitting them out
//! lets the TUI and other consumers type-check against the trait surface in
//! parallel with the server crate instead of waiting on it.

pub mod client;
pub mod transcript;

pub use client::{AppClient, AppEventReceiver, AppNotificationReceiver};
