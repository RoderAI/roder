//! Roder Chrome browser-control extension.
//!
//! Registers the model-facing `chrome_*` tools and a policy contributor that
//! gates protected and prohibited browser actions. The tools are generic over an
//! injected [`roder_api::chrome::ChromeController`] (defaulting to the live
//! process bridge), so they can be unit-tested against a fake bridge without a
//! real browser.
//!
//! [`direct`] is Roder's own direct CDP toolset, bound to one tab: the
//! `chrome_*` tools use it for Roder Desktop's integrated browser when no
//! extension is connected, and Jev binds it to its session's tab as
//! `jev_tab_*` (see `roder-ext-jev`).
//!
//! This crate deliberately depends only on `roder-api` (not `roder-core`): the
//! shared browser-bridge contract lives in [`roder_api::chrome`].

mod artifacts;
mod chrome_select;
mod computer;
mod desktop_cdp;
mod desktop_outcome;
mod desktop_scope;
mod desktop_tabs;
pub mod direct;
mod extension;
mod extension_result;
mod observed;
mod observed_cut;
mod observed_read;
mod observed_render;
mod policy;
mod session;
mod tools;

pub use artifacts::{ChromeArtifact, ChromeArtifactKind};
pub use computer::{ComputerCdpBinding, ComputerTool, ComputerToolContributor};
pub use extension::ChromeExtension;
pub use policy::{ChromeActionClass, classify_action};
pub use tools::{ChromeToolContributor, chrome_tool_specs};
