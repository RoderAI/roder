//! What the model reads after a `chrome_*` action through the paired
//! extension: the outcome first, then the controls, then the text, within the
//! output cap; and, for a build that answers an action without observing the
//! page, one `page/snapshot` after a short delay. No browser and no model: the
//! extension is scripted.
use std::sync::Arc;
use std::time::Duration;

use roder_api::{
    chrome::{ChromeCommand, ChromeController, ChromeError, ChromePermissionMode, ChromeStatus},
    policy_mode::PolicyMode,
    tools::{ToolCall, ToolContributor, ToolExecutionContext, ToolRegistry, ToolResult},
};
use roder_ext_chrome::ChromeToolContributor;
use serde_json::{Value, json};

include!("support/scripted.rs");
use scripted::{Scripted, control, observed_reply, snapshot, snapshot_reply};

/// The most characters one page result carries, and the most lines: both a
/// margin inside the runtime's own cut of a tool result (20,000 characters,
/// 200 lines), which would keep only the two ends of a longer one.
const CAP: usize = 18_000;
const LINES: usize = 150;

fn registry(extension: &Arc<Scripted>) -> ToolRegistry {
    let mut registry = ToolRegistry::default();
    ChromeToolContributor::with_controller(extension.clone())
        .contribute(&mut registry)
        .unwrap();
    registry
}

async fn call(registry: &ToolRegistry, name: &str, args: Value) -> ToolResult {
    registry
        .get(name)
        .unwrap()
        .execute(
            ToolExecutionContext::new("thread", "turn", PolicyMode::Default),
            ToolCall {
                id: name.into(),
                name: name.into(),
                raw_arguments: args.to_string(),
                arguments: args,
                thread_id: "thread".into(),
                turn_id: "turn".into(),
            },
        )
        .await
        .unwrap()
}

/// The sentence on the result's `Outcome:` line.
fn outcome(text: &str) -> String {
    text.lines()
        .find_map(|line| line.strip_prefix("Outcome: "))
        .unwrap_or_else(|| panic!("no Outcome line in:\n{text}"))
        .to_string()
}

fn at(text: &str, needle: &str) -> usize {
    text.find(needle)
        .unwrap_or_else(|| panic!("{needle:?} not in:\n{text}"))
}

fn line_of<'a>(text: &'a str, start: &str) -> &'a str {
    text.lines()
        .find(|line| line.starts_with(start))
        .unwrap_or_else(|| panic!("no line starting {start:?} in:\n{text}"))
}

fn cart(controls: Vec<Value>) -> Value {
    snapshot("https://app.test/cart", "Cart", "Your cart", controls)
}

// The tests are grouped by behaviour in `extension_observation/`; they share
// this file's helpers and imports, so they are included here, not compiled as
// modules (a test keeps the name it had when the groups were one file).
include!("extension_observation/outcomes.rs");
include!("extension_observation/old_build.rs");
include!("extension_observation/budget.rs");
include!("extension_observation/snapshots.rs");
include!("extension_observation/same_tab.rs");
