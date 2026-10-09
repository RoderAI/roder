//! Integration tests against a fake browser-use MCP server.
//!
//! The fake is this test binary itself: `fake_server_entry` turns into a
//! stdio MCP server when `RODER_FAKE_BROWSER_USE=1`, answering with the
//! pinned browser-use tool list. It also starts a long-lived child process to
//! stand in for the browser, so the tests can check that stopping the server
//! stops its whole process tree. No uvx, Python or network is needed.
//!
//! The tests live in `fake_server/`, one module per behaviour group:
//! `lifecycle` (start, reuse, stop, cancel), `ordering` (report and
//! observation), `loss` (what a lost or kept browser says), `state_view` (the
//! compact page state) and `selects` (native selects). `support` launches the
//! fake and runs tool calls, and `fake` is the server itself.

#[path = "support/compact_state.rs"]
mod compact_state;
#[path = "fake_server/fake.rs"]
mod fake;
#[path = "fake_server/lifecycle.rs"]
mod lifecycle;
#[path = "fake_server/loss.rs"]
mod loss;
#[path = "fake_server/ordering.rs"]
mod ordering;
#[path = "fake_server/selects.rs"]
mod selects;
#[path = "fake_server/state_view.rs"]
mod state_view;
#[path = "fake_server/support.rs"]
mod support;

/// Entry point of the fake server; a no-op in a normal test run.
#[test]
fn fake_server_entry() {
    if std::env::var(fake::FAKE_ENV).as_deref() != Ok("1") {
        return;
    }
    fake::run_fake_server();
    std::process::exit(0);
}
