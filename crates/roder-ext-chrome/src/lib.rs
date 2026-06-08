//! Roder Chrome browser-control extension.
//!
//! Registers the model-facing `chrome_*` tools and a policy contributor that
//! gates protected and prohibited browser actions. The tools are generic over an
//! injected [`roder_api::chrome::ChromeController`] (defaulting to the live
//! process bridge), so they can be unit-tested against a fake bridge without a
//! real browser.
//!
//! This crate deliberately depends only on `roder-api` (not `roder-core`): the
//! shared browser-bridge contract lives in [`roder_api::chrome`].

mod artifacts;
mod desktop_cdp;
mod extension;
mod policy;
mod session;
mod tools;

pub use artifacts::{ChromeArtifact, ChromeArtifactKind};
pub use extension::ChromeExtension;
pub use policy::{ChromeActionClass, classify_action};
pub use tools::{ChromeToolContributor, chrome_tool_specs};
