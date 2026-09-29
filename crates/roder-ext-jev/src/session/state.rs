//! What a session remembers between calls.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;

use serde::Serialize;

use super::SessionTabs;
use crate::chrome::ChromeEndpoint;
use crate::engine::{JevDecisionClient, JevRunResult, JevStatus, JevTextValueResolver};
use crate::secret::Secrets;
use crate::text_helper::TextHelper;

/// Earlier calls a session keeps.
pub(crate) const TRAIL_CALLS: usize = 8;
const GOAL_CHARS: usize = 200;
const TITLE_CHARS: usize = 120;
const REASON_CHARS: usize = 200;

/// One finished call, as later results recall it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct CallRecord {
    /// The call's number in the session, from 1.
    pub(crate) n: u32,
    pub(crate) goal: String,
    /// The url it was given, or empty when it went on from the tab's page.
    pub(crate) start: String,
    pub(crate) end_url: String,
    pub(crate) title: String,
    pub(crate) status: JevStatus,
    pub(crate) actions: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) stopped_because: Option<String>,
}

/// Running counts over a session's calls.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(crate) struct Totals {
    pub(crate) calls: u32,
    pub(crate) actions: usize,
    pub(crate) decisions: usize,
    pub(crate) text_calls: usize,
    /// Tool calls the automatic fallback made in this session's calls.
    pub(crate) fallback_actions: usize,
    /// Calls to the `jev_tab_*` tools on this session's tab.
    pub(crate) tab_tool_calls: usize,
}

/// The models a session's calls use, resolved once and kept until the
/// turn's model changes.
pub(crate) struct SessionModels {
    /// What they were resolved for; a call with another key resolves again.
    pub(crate) key: String,
    pub(crate) decision: Arc<dyn JevDecisionClient>,
    pub(crate) text: Option<Arc<dyn JevTextValueResolver>>,
    /// Roder's own text helper, whose current model the result reports.
    pub(crate) helper: Option<Arc<TextHelper>>,
}

pub(crate) struct SessionState {
    /// The browser the tabs live in, as last resolved.
    pub(crate) endpoint: Option<ChromeEndpoint>,
    pub(crate) tabs: SessionTabs,
    pub(crate) trail: VecDeque<CallRecord>,
    pub(crate) totals: Totals,
    /// Every secret typed in this session, scrubbed from later page reads.
    pub(crate) secrets: Secrets,
    pub(crate) models: Option<SessionModels>,
    pub(crate) last_used: Instant,
    /// Taken out of the registry; a call that finds it so starts over.
    pub(crate) closed: bool,
    /// The local date the session began on, as [`crate::report::date`]
    /// gives it: what "tonight" meant when the user asked, should a flow
    /// run past midnight.
    pub(crate) began_on: String,
}

impl SessionState {
    pub(crate) fn new(max_tabs: usize) -> Self {
        Self {
            endpoint: None,
            tabs: SessionTabs::new(max_tabs),
            trail: VecDeque::new(),
            totals: Totals::default(),
            secrets: Secrets::default(),
            models: None,
            last_used: Instant::now(),
            closed: false,
            began_on: crate::report::date(),
        }
    }

    /// Count a finished call and remember it for the next ones.
    pub(crate) fn record(&mut self, goal: &str, start: Option<&str>, result: &JevRunResult) {
        self.totals.calls += 1;
        self.totals.actions += result.actions.len();
        self.totals.decisions += result.model_calls;
        self.totals.text_calls += result.text_calls;
        self.secrets.extend(&result.typed_secrets);
        self.trail.push_back(CallRecord {
            n: self.totals.calls,
            goal: cut(&self.secrets.scrub(goal), GOAL_CHARS),
            start: start.unwrap_or_default().to_string(),
            end_url: result.url.clone(),
            title: cut(&result.title, TITLE_CHARS),
            status: result.status,
            actions: result.actions.len(),
            stopped_because: result
                .stopped_because
                .as_deref()
                .map(|reason| cut(reason, REASON_CHARS)),
        });
        while self.trail.len() > TRAIL_CALLS {
            self.trail.pop_front();
        }
        self.last_used = Instant::now();
    }
}

/// `text` cut to `chars` characters, marked when cut.
pub(crate) fn cut(text: &str, chars: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= chars {
        return text.to_string();
    }
    let mut kept = text
        .chars()
        .take(chars.saturating_sub(1))
        .collect::<String>();
    kept.push('…');
    kept
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn result(status: JevStatus, url: &str) -> JevRunResult {
        JevRunResult::before_start(status, url, Duration::ZERO, "stopped")
    }

    #[test]
    fn the_trail_keeps_the_last_eight_calls_and_counts_them_all() {
        let mut state = SessionState::new(3);
        for n in 1..=10 {
            state.record(
                &format!("goal {n}"),
                (n == 1).then_some("https://a.test/"),
                &result(JevStatus::Done, &format!("https://a.test/{n}")),
            );
        }
        assert_eq!(state.totals.calls, 10);
        assert_eq!(state.trail.len(), TRAIL_CALLS);
        assert_eq!(state.trail.front().unwrap().n, 3);
        assert_eq!(state.trail.back().unwrap().goal, "goal 10");
        assert_eq!(state.trail.back().unwrap().start, "");
    }

    #[test]
    fn long_goals_are_cut_and_typed_secrets_scrubbed_from_them() {
        let mut state = SessionState::new(3);
        let mut typed = result(JevStatus::Done, "https://a.test/");
        typed.typed_secrets.remember("hunter22");
        state.record("Sign in with hunter22", None, &typed);
        assert_eq!(state.trail[0].goal, "Sign in with [secret]");
        // A later run that typed nothing keeps the earlier secret.
        state.record(
            &"x".repeat(500),
            None,
            &result(JevStatus::Done, "https://a.test/"),
        );
        assert_eq!(state.trail[1].goal.chars().count(), GOAL_CHARS);
        assert!(state.trail[1].goal.ends_with('…'));
        assert_eq!(state.secrets.scrub("hunter22"), "[secret]");
    }
}
