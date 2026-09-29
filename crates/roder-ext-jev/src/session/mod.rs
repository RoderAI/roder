//! One Jev browser session per thread.
//!
//! A thread's `jev_browse` calls share a session: its tabs (by target id,
//! at most three, the last current), where the last call left off, the
//! calls so far, the secrets typed and the resolved models. A call goes on
//! in the session's current tab rather than opening one, and the tabs stay
//! open between calls. The registry is process-wide and keyed by thread id,
//! like `roder_api::mcp_auth`, because a subagent host builds its own tool
//! registry; a subagent's thread has its own session. Calls on one session
//! run one at a time, and sessions of different threads side by side.
//!
//! A session leaves the registry only while its state lock is held, and is
//! marked closed as it goes: a call that got hold of it before and waited
//! for the lock finds it closed and starts over on the thread's current
//! session, so no call runs on a session the registry no longer lists,
//! whose tabs nothing would ever close.

mod call;
pub(crate) mod log;
mod state;
mod sweep;
mod tabs;

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;

use crate::cdp::Connection;
use crate::chrome::ChromeEndpoint;
use crate::runner::{close_all, close_quietly};
#[cfg(test)]
pub(crate) use call::BUSY;
pub(crate) use state::{SessionModels, SessionState, cut};
use sweep::{DiskLedger, recordable};
pub(crate) use tabs::{SessionTabs, TabNote};

/// Idle time after which a session's tabs are closed, by default.
const DEFAULT_IDLE: Duration = Duration::from_secs(20 * 60);
/// How often the sweeper looks for idle sessions.
const SWEEP_EVERY: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy)]
pub(crate) struct SessionLimits {
    /// `JEV_SESSION_IDLE_SECS`: a session idle this long is closed.
    pub(crate) idle: Duration,
    /// Sessions one process keeps; a new one pushes out the least recently
    /// used idle one.
    pub(crate) max_sessions: usize,
    /// Tabs one session keeps open.
    pub(crate) max_tabs: usize,
}

impl SessionLimits {
    pub(crate) fn from_env() -> Self {
        Self {
            idle: idle(std::env::var("JEV_SESSION_IDLE_SECS").ok().as_deref()),
            max_sessions: 8,
            max_tabs: 3,
        }
    }
}

/// `JEV_SESSION_IDLE_SECS`, a positive whole number of seconds; anything
/// else is the default 20 minutes.
fn idle(raw: Option<&str>) -> Duration {
    raw.and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .map_or(DEFAULT_IDLE, Duration::from_secs)
}

/// What can be read of a session without waiting for a call on it.
#[derive(Debug, Clone)]
pub(crate) struct SessionSummary {
    /// Where the current tab was when the last call ended.
    pub(crate) current_url: Option<String>,
    pub(crate) targets: Vec<String>,
    /// The browser, when a ledger may name it (loopback only).
    pub(crate) endpoint: Option<String>,
    pub(crate) last_used: Instant,
}

impl SessionSummary {
    fn empty() -> Self {
        Self {
            current_url: None,
            targets: Vec::new(),
            endpoint: None,
            last_used: Instant::now(),
        }
    }
}

pub(crate) struct JevSession {
    state: Arc<tokio::sync::Mutex<SessionState>>,
    summary: RwLock<SessionSummary>,
}

impl JevSession {
    fn new(max_tabs: usize) -> Self {
        Self {
            state: Arc::new(tokio::sync::Mutex::new(SessionState::new(max_tabs))),
            summary: RwLock::new(SessionSummary::empty()),
        }
    }

    fn summary(&self) -> SessionSummary {
        self.summary
            .read()
            .map(|summary| summary.clone())
            .unwrap_or_else(|_| SessionSummary::empty())
    }

    /// Publish what `state` now holds for readers that must not wait.
    fn publish(&self, state: &SessionState) {
        let summary = SessionSummary {
            current_url: state.tabs.last_url.clone(),
            targets: state.tabs.targets(),
            endpoint: state.endpoint.as_ref().and_then(recordable),
            last_used: state.last_used,
        };
        if let Ok(mut shared) = self.summary.write() {
            *shared = summary;
        }
    }
}

/// What a session call needs from outside: models and a browser. Roder's
/// own resolves them from the environment; tests inject their own.
#[async_trait]
pub(crate) trait SessionDeps: Send + Sync {
    /// What the models are resolved for; the session keeps its models
    /// while this stays the same.
    fn model_key(&self) -> String;
    async fn models(&self) -> anyhow::Result<SessionModels>;
    async fn endpoint(&self) -> anyhow::Result<ChromeEndpoint>;
}

pub(crate) struct JevSessions {
    sessions: Mutex<HashMap<String, Arc<JevSession>>>,
    limits: SessionLimits,
    sweeper: OnceLock<()>,
    ledger: Option<DiskLedger>,
}

impl JevSessions {
    /// The process's registry, with a ledger in Roder's config directory.
    pub(crate) fn global() -> &'static JevSessions {
        static GLOBAL: OnceLock<JevSessions> = OnceLock::new();
        GLOBAL.get_or_init(|| {
            let mut sessions = Self::new(SessionLimits::from_env());
            sessions.ledger = Some(DiskLedger::new(
                roder_config::config_dir().join("jev-sessions"),
            ));
            sessions
        })
    }

    pub(crate) fn new(limits: SessionLimits) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            limits,
            sweeper: OnceLock::new(),
            ledger: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_ledger(mut self, dir: std::path::PathBuf) -> Self {
        self.ledger = Some(DiskLedger::new(dir));
        self
    }

    fn map(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<JevSession>>> {
        self.sessions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// `thread`'s session as it stands, without waiting for a call on it.
    pub(crate) fn peek(&self, thread: &str) -> Option<SessionSummary> {
        self.map().get(thread).map(|session| session.summary())
    }

    /// Whether `thread`'s session has a tab to go on in.
    pub(crate) fn has_tab(&self, thread: &str) -> bool {
        self.peek(thread)
            .is_some_and(|summary| !summary.targets.is_empty())
    }

    /// How many sessions are open, for tests.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.map().len()
    }

    /// `thread`'s session, created if it has none. A new session past the
    /// cap pushes out the least recently used one no call is using.
    pub(crate) async fn get_or_create(&self, thread: &str) -> Arc<JevSession> {
        let (session, evicted) = {
            let mut map = self.map();
            if let Some(session) = map.get(thread) {
                return session.clone();
            }
            let mut evicted = None;
            if map.len() >= self.limits.max_sessions {
                let mut idle = map
                    .iter()
                    .map(|(thread, session)| (session.summary().last_used, thread.clone()))
                    .collect::<Vec<_>>();
                idle.sort();
                evicted = idle
                    .into_iter()
                    .find_map(|(_, oldest)| retire_if_idle(&mut map, &oldest));
            }
            let session = Arc::new(JevSession::new(self.limits.max_tabs));
            map.insert(thread.to_string(), session.clone());
            (session, evicted)
        };
        if let Some(evicted) = evicted {
            evicted.close().await;
        }
        session
    }

    /// `thread`'s session, locked, waiting until `deadline`; `None` when the
    /// wait ran out. A session closed while this call waited for it is
    /// passed over for the one the thread has now.
    pub(crate) async fn lock(
        &self,
        thread: &str,
        deadline: tokio::time::Instant,
    ) -> Option<(Arc<JevSession>, SessionGuard)> {
        loop {
            let session = self.get_or_create(thread).await;
            let state = tokio::time::timeout_at(deadline, session.state.clone().lock_owned())
                .await
                .ok()?;
            if !state.closed {
                return Some((session, state));
            }
        }
    }

    /// Close `thread`'s tabs and forget its session; how many tabs were
    /// closed. Waits, until `deadline`, for a call still running on it.
    pub(crate) async fn close(
        &self,
        thread: &str,
        deadline: tokio::time::Instant,
    ) -> anyhow::Result<usize> {
        let Some(session) = self.map().get(thread).cloned() else {
            return Ok(0);
        };
        let mut state = tokio::time::timeout_at(deadline, session.state.lock())
            .await
            .map_err(|_| anyhow::anyhow!(call::BUSY))?;
        if state.closed {
            // Closed by the sweeper or another close while this one waited.
            return Ok(0);
        }
        let retired = retire(&mut state);
        session.publish(&state);
        {
            let mut map = self.map();
            if map
                .get(thread)
                .is_some_and(|current| Arc::ptr_eq(current, &session))
            {
                map.remove(thread);
            }
        }
        drop(state);
        let closed = retired.close().await;
        self.write_ledger();
        Ok(closed)
    }

    /// Close the sessions idle past the limit, refresh this process's
    /// ledger, and sweep dead processes' ones.
    pub(crate) async fn sweep(&self) {
        let expired = {
            let mut map = self.map();
            let idle = map
                .iter()
                .filter(|(_, session)| session.summary().last_used.elapsed() >= self.limits.idle)
                .map(|(thread, _)| thread.clone())
                .collect::<Vec<_>>();
            idle.iter()
                .filter_map(|thread| retire_if_idle(&mut map, thread))
                .collect::<Vec<_>>()
        };
        for retired in expired {
            retired.close().await;
        }
        self.write_ledger();
        if let Some(ledger) = &self.ledger {
            ledger.sweep_dead(self.limits.idle * 2).await;
        }
    }

    /// Run [`sweep`](Self::sweep) every minute from now on, once per
    /// registry, when a Tokio runtime is there to run it.
    pub(crate) fn start_sweeper(&'static self) {
        self.sweeper.get_or_init(|| {
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    let mut every = tokio::time::interval(SWEEP_EVERY);
                    every.tick().await;
                    loop {
                        every.tick().await;
                        self.sweep().await;
                    }
                });
            }
        });
    }

    /// Record every session's tabs in this process's ledger.
    fn write_ledger(&self) {
        let Some(ledger) = &self.ledger else {
            return;
        };
        let mut browsers = BTreeMap::<String, Vec<String>>::new();
        for session in self.map().values() {
            let summary = session.summary();
            if let Some(endpoint) = summary.endpoint {
                browsers
                    .entry(endpoint)
                    .or_default()
                    .extend(summary.targets);
            }
        }
        ledger.write(&browsers);
    }
}

/// A session's state lock, held across a whole call.
pub(crate) type SessionGuard = tokio::sync::OwnedMutexGuard<SessionState>;

/// Take `thread`'s session out of `map` when no call holds it, marked
/// closed under its lock; the tabs it had, to close. A session a call is
/// using, or waiting for, stays.
fn retire_if_idle(map: &mut HashMap<String, Arc<JevSession>>, thread: &str) -> Option<Retired> {
    let session = map.get(thread)?;
    let mut state = session.state.try_lock().ok()?;
    let retired = retire(&mut state);
    session.publish(&state);
    drop(state);
    map.remove(thread);
    Some(retired)
}

/// Mark `state` closed and take its tabs from it.
fn retire(state: &mut SessionState) -> Retired {
    state.closed = true;
    let targets = state.tabs.targets();
    state.tabs.clear();
    Retired {
        endpoint: state
            .endpoint
            .as_ref()
            .map(|endpoint| endpoint.url().to_string()),
        targets,
    }
}

/// The tabs of a session that has left the registry, still to close.
struct Retired {
    endpoint: Option<String>,
    targets: Vec<String>,
}

impl Retired {
    /// Close the tabs; how many there were.
    async fn close(self) -> usize {
        if let Some(endpoint) = &self.endpoint
            && !self.targets.is_empty()
        {
            close_quietly(async {
                let mut connection = Connection::connect(endpoint).await?;
                close_all(&mut connection, &self.targets).await;
                Ok(())
            })
            .await;
        }
        self.targets.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_idle_limit_is_twenty_minutes_unless_set() {
        assert_eq!(idle(None), Duration::from_secs(1200));
        assert_eq!(idle(Some(" 90 ")), Duration::from_secs(90));
        assert_eq!(idle(Some("0")), Duration::from_secs(1200));
        assert_eq!(idle(Some("soon")), Duration::from_secs(1200));
    }

    #[tokio::test]
    async fn sessions_are_per_thread_and_the_oldest_idle_one_goes_first() {
        let sessions = JevSessions::new(SessionLimits {
            idle: Duration::from_secs(60),
            max_sessions: 2,
            max_tabs: 3,
        });
        let a = sessions.get_or_create("a").await;
        assert!(Arc::ptr_eq(&a, &sessions.get_or_create("a").await));
        tokio::time::sleep(Duration::from_millis(5)).await;
        sessions.get_or_create("b").await;
        // "a" is busy, so "b" goes even though it is newer.
        let busy = a.state.lock().await;
        sessions.get_or_create("c").await;
        assert!(sessions.peek("a").is_some());
        assert!(sessions.peek("b").is_none());
        drop(busy);
        sessions.get_or_create("d").await;
        assert!(sessions.peek("a").is_none(), "a is now the oldest idle one");
        assert!(a.state.lock().await.closed, "evicted under its lock");
        assert_eq!(sessions.len(), 2);
        assert!(!sessions.has_tab("c"));
    }

    fn far() -> tokio::time::Instant {
        tokio::time::Instant::now() + Duration::from_secs(5)
    }

    /// A call that got the session before a close did, and waited for its
    /// lock behind the close, goes on in the thread's new session, never in
    /// the closed one the registry no longer lists.
    #[tokio::test]
    async fn a_call_waiting_behind_a_close_moves_to_the_new_session() {
        let sessions = Arc::new(JevSessions::new(SessionLimits::from_env()));
        let (old, running) = sessions.lock("t", far()).await.unwrap();
        let closing = tokio::spawn({
            let sessions = sessions.clone();
            async move { sessions.close("t", far()).await }
        });
        tokio::task::yield_now().await;
        let waiting = tokio::spawn({
            let sessions = sessions.clone();
            async move {
                let (session, state) = sessions.lock("t", far()).await.unwrap();
                (session, state.closed)
            }
        });
        tokio::task::yield_now().await;
        drop(running);
        closing.await.unwrap().unwrap();
        let (session, closed) = waiting.await.unwrap();
        assert!(!closed);
        assert!(!Arc::ptr_eq(&session, &old), "moved off the closed session");
        assert!(old.state.lock().await.closed);
        let listed = sessions.map().get("t").cloned().unwrap();
        assert!(Arc::ptr_eq(&listed, &session), "the registry lists it");
    }

    /// The sweeper takes a session out only under its lock: one a call
    /// holds stays listed, and one it takes is closed for any call that
    /// still has it.
    #[tokio::test]
    async fn the_sweeper_retires_only_sessions_no_call_holds() {
        let sessions = JevSessions::new(SessionLimits {
            idle: Duration::from_millis(1),
            max_sessions: 8,
            max_tabs: 3,
        });
        let (held, guard) = sessions.lock("held", far()).await.unwrap();
        let idle = sessions.get_or_create("idle").await;
        tokio::time::sleep(Duration::from_millis(5)).await;
        sessions.sweep().await;
        assert!(sessions.peek("held").is_some());
        assert!(sessions.peek("idle").is_none());
        assert!(idle.state.lock().await.closed);
        drop(guard);
        let (again, _) = sessions.lock("idle", far()).await.unwrap();
        assert!(!Arc::ptr_eq(&again, &idle));
        assert!(Arc::ptr_eq(&sessions.get_or_create("held").await, &held));
    }
}
