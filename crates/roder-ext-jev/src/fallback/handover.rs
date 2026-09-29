//! The hand-over: Roder's full browser tools, registered as `jev_tab_*`,
//! driving this thread's Jev tab and nothing else.
//!
//! They are always registered with `jev_browse` (no `--chrome` needed), and
//! each call acts in the tab the thread's Jev session is on, under the same
//! session lock `jev_browse` takes, so a tool call and a Jev call never
//! drive the tab at once. A thread without a Jev tab gets an error telling
//! it to start with `jev_browse`; the tools cannot name another tab, open
//! one, or reach the user's own browser. Jev's rules come with the tab
//! (see [`super::guard`]), and what a call does is recorded on the session:
//! a tab it opened is adopted, a secret it typed is scrubbed from later
//! reads, and where it left the tab is where Jev's next call goes on.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use roder_api::tools::{ToolCall, ToolExecutionContext, ToolExecutor};
use roder_ext_chrome::direct::{
    DirectBinding, DirectGuard, DirectLease, DirectStep, DirectTab, direct_tools,
};
use serde_json::json;

use super::guard::JevGuard;
use super::run::PREFIX;
use crate::runner::{Ceilings, close_all, close_quietly};
use crate::session::{JevSession, JevSessions, SessionGuard, log as session_log};

/// How long a tool call waits for a `jev_browse` call on the same tab.
const LOCK_WAIT: Duration = Duration::from_secs(30);

/// What a tool call on a thread without a Jev tab reads.
pub(crate) const NO_JEV_TAB: &str = "This thread has no Jev tab: the jev_tab_* tools only drive \
     the tab a jev_browse call opened. Call jev_browse with the url to start on first.";

/// The hand-over tools, bound to the process's Jev sessions.
pub(crate) fn tools() -> Vec<Arc<dyn ToolExecutor>> {
    tools_on(JevSessions::global())
}

/// The hand-over tools on `sessions` (a test's own registry).
pub(crate) fn tools_on(sessions: &'static JevSessions) -> Vec<Arc<dyn ToolExecutor>> {
    direct_tools(
        PREFIX,
        "this thread's Jev browser tab",
        Arc::new(JevTabs { sessions }),
    )
}

/// The tool names, for results that tell the caller about them.
pub(crate) fn tool_names() -> Vec<String> {
    roder_ext_chrome::direct::DIRECT_TOOLS
        .iter()
        .map(|short| format!("{PREFIX}_{short}"))
        .collect()
}

struct JevTabs {
    sessions: &'static JevSessions,
}

#[async_trait]
impl DirectBinding for JevTabs {
    async fn lease(
        &self,
        ctx: &ToolExecutionContext,
        _call: &ToolCall,
    ) -> Result<Box<dyn DirectLease>, String> {
        let sessions = self.sessions;
        if !sessions.has_tab(&ctx.thread_id) {
            return Err(NO_JEV_TAB.into());
        }
        let wait = ctx.deadline_remaining_seconds.map_or(LOCK_WAIT, |seconds| {
            LOCK_WAIT.min(Duration::from_secs(seconds))
        });
        let Some((session, state)) = sessions
            .lock(&ctx.thread_id, tokio::time::Instant::now() + wait)
            .await
        else {
            return Err(crate::session::BUSY.into());
        };
        let (Some(endpoint), Some(current)) = (state.endpoint.clone(), state.tabs.current()) else {
            return Err(NO_JEV_TAB.into());
        };
        let ceilings = Ceilings::from_env().map_err(|error| format!("{error:#}"))?;
        let guard = Arc::new(JevGuard::new(
            ceilings.scope,
            ceilings.confirm_irreversible,
            ceilings.refuse_cookie_banners,
            state.secrets.clone(),
        ));
        let tab = DirectTab::Target {
            endpoint: endpoint.url().to_string(),
            target_id: current.target.clone(),
        };
        Ok(Box::new(JevTabLease {
            thread: ctx.thread_id.clone(),
            sessions,
            session,
            state,
            guard,
            tab,
        }))
    }
}

struct JevTabLease {
    thread: String,
    sessions: &'static JevSessions,
    session: Arc<JevSession>,
    state: SessionGuard,
    guard: Arc<JevGuard>,
    tab: DirectTab,
}

#[async_trait]
impl DirectLease for JevTabLease {
    fn tab(&self) -> DirectTab {
        self.tab.clone()
    }

    fn guard(&self) -> Arc<dyn DirectGuard> {
        self.guard.clone()
    }

    /// The policy asks the user about every call that sets
    /// `authorize_irreversible`, in every mode but bypass.
    fn may_authorize(&self) -> bool {
        true
    }

    async fn finish(self: Box<Self>, step: &DirectStep, _target_id: &str) {
        let Self {
            thread,
            sessions,
            session,
            mut state,
            ..
        } = *self;
        if let Some(secret) = &step.typed_secret {
            state.secrets.remember(secret);
        }
        let mut closing = Vec::new();
        if let Some(opened) = &step.opened_tab {
            state.tabs.adopt(&opened.target_id, &opened.opener);
            closing = state.tabs.cap();
        }
        if let Some(url) = step.data["page"]["url"].as_str() {
            state.tabs.last_url = Some(url.to_string());
        }
        state.totals.tab_tool_calls += 1;
        state.last_used = std::time::Instant::now();
        if let (Some(endpoint), false) = (state.endpoint.clone(), closing.is_empty()) {
            close_quietly(async {
                let mut connection = crate::cdp::Connection::connect(endpoint.url()).await?;
                close_all(&mut connection, &closing).await;
                Ok(())
            })
            .await;
        }
        session.publish(&state);
        drop(state);
        sessions.write_ledger();
        if let Some(dir) = session_log::dir() {
            let mut data = step.data.clone();
            data["is_error"] = json!(step.is_error);
            session_log::append_tab_tool(
                &dir,
                &thread,
                data["tool"].as_str().unwrap_or_default(),
                &data,
                &step.text,
            );
        }
    }
}
