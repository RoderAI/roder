//! Injectable JEV execution primitives.
//!
//! The built-in `jev_browse` tool connects these interfaces to Chrome and the
//! configured model providers. Hosted runners can instead keep browser
//! ownership, evidence capture, and secret handling in their supervisor while
//! reusing the same bounded JEV decision loop.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::agent::Agent;
use crate::prompts::MAX_STEPS;
use crate::scope::JevOriginScope;
use crate::secret::Secrets;
pub use covered::Covered;
pub use records::{
    JevActionRecord, JevControl, JevDecisionRecord, JevFrameText, JevOmitted, JevPageFacts,
    JevRunResult, JevSuppressedClick, JevSuppressedKind,
};

/// A stale observation means the browser changed after JEV made its decision.
#[derive(Debug)]
pub struct StaleObservation(String);

impl StaleObservation {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl std::fmt::Display for StaleObservation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

impl std::error::Error for StaleObservation {}

/// What an executed action came to. An action that ran but that the page
/// did not keep is still a step: the loop records why on it and goes on, so
/// the stall rule, not an error, ends a run the page keeps refusing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JevActOutcome {
    /// Why the page did not keep what the action asked for: a select it put
    /// back, or a fill whose text did not stay in the field.
    pub refused: Option<String>,
    /// The target was covered by a popover, menu or dialog, and the browser
    /// dismissed it first (how: Escape, its close control, or a press
    /// outside it) before acting.
    pub uncovered: Option<String>,
}

impl JevActOutcome {
    /// The action ran as asked.
    pub fn done() -> Self {
        Self::default()
    }

    /// The action ran, but the page did not keep its effect.
    pub fn refused(reason: impl Into<String>) -> Self {
        Self {
            refused: Some(reason.into()),
            ..Self::default()
        }
    }
}

/// A JavaScript dialog the page opened and Jev answered: an `alert` or
/// `beforeunload` is accepted, a `confirm` or `prompt` dismissed. Its message
/// is page text, and as untrusted as the rest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JevDialog {
    #[serde(rename = "type")]
    pub kind: String,
    pub message: String,
    pub accepted: bool,
}

mod browser;
pub use browser::JevBrowser;

/// One model decision in the JEV action space.
#[derive(Debug, Clone)]
pub struct JevDecision {
    pub choice: String,
    pub operation: String,
    pub target: Option<String>,
    /// The operation head's confidence.
    pub confidence: f64,
    /// The target head's confidence, for an operation that has targets.
    pub target_confidence: Option<f64>,
    /// The deciding head's distribution, keyed by observed action id: the
    /// target head's for a targeted operation, else the operation head's.
    pub probabilities: Map<String, Value>,
    pub latency_ms: u64,
    pub usage: Value,
    /// The model version that answered, as the service reports it.
    pub model: Option<String>,
    /// With the irreversible-action gate on, P(the chosen action cannot be
    /// undone) from the question asked about it in the same request. `None`
    /// when it was not asked, or its answer was missing or invalid; the gate
    /// treats an action it applies to with no answer as irreversible.
    pub irreversible: Option<f64>,
}

impl JevDecision {
    /// The least certain judgement behind the call: an action is only as
    /// sure as the operation and the target that chose it.
    pub fn call_confidence(&self) -> f64 {
        self.target_confidence
            .map_or(self.confidence, |target| target.min(self.confidence))
    }

    pub fn probability_of(&self, choice: &str) -> Value {
        self.probabilities
            .get(choice)
            .cloned()
            .unwrap_or(Value::Null)
    }
}

/// Chooses the next action from an observation produced by [`JevBrowser`].
#[async_trait]
pub trait JevDecisionClient: Send + Sync {
    /// Whether the client can use a viewport image alongside observed actions.
    /// When available and allowed by secret handling, the engine supplies an
    /// inline data URL in `observation["_screenshot"]` for this decision only.
    /// Wrapping clients should forward this capability.
    fn uses_images(&self) -> bool {
        false
    }

    async fn choose(
        &self,
        observation: &Value,
        goal: &str,
        history: &[Value],
    ) -> anyhow::Result<JevDecision>;

    /// [`choose`](Self::choose) with the irreversible-action gate on: the
    /// same request also asks, for each offered action that may commit
    /// something, whether it would do what cannot be undone, and the decision
    /// carries the chosen action's answer in [`JevDecision::irreversible`].
    /// The loop calls this instead of `choose` when
    /// [`JevEngineConfig::with_irreversible_gate`] is set. The default asks
    /// nothing, so the gate treats every such action a client without it
    /// chooses as irreversible; a wrapping client should forward it.
    async fn choose_gated(
        &self,
        observation: &Value,
        goal: &str,
        history: &[Value],
    ) -> anyhow::Result<JevDecision> {
        self.choose(observation, goal, history).await
    }
}

/// Sends a provider-native decision request and returns its JSON response.
/// The constructing client determines whether the wire format is TypeSafe or
/// OpenAI Decisions; relays must preserve that provider contract.
///
/// Hosted runners can use this seam to route decisions through an authenticated
/// control plane without moving provider credentials into the runner.
#[async_trait]
pub trait JevDecisionTransport: Send + Sync {
    async fn decide(&self, request: &Value) -> anyhow::Result<Value>;
}

/// A value generated for a fill action.
#[derive(Debug, Clone)]
pub struct JevTextValue {
    pub value: String,
    pub model: String,
    pub latency_ms: u64,
    pub usage: Value,
}

/// Resolves a fill value from JEV's field context.
///
/// The preferred source for a password or one-time code (the field
/// context's `field.input_type` is `password` or `one-time-code`):
/// supervisors may return an opaque value reference and resolve it only
/// while executing the browser action, so the secret never enters the goal,
/// the decision request, the text model or the JEV trace. Whatever is
/// returned for a secret field is recorded as `[secret]`. A resolver with
/// no value should fail with [`JevStop`] and [`JevStatus::NeedsInput`],
/// naming the field; Roder's own text helper does, and never types a
/// password or code that is not written in the goal.
#[async_trait]
pub trait JevTextValueResolver: Send + Sync {
    async fn resolve(&self, field_context: &Value) -> anyhow::Result<JevTextValue>;
}

/// Where a run stands. Every status but `Ready` ends it; `Ready` is only seen
/// mid-run. New statuses may be added as something comes to produce them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum JevStatus {
    Ready,
    /// The model answered DONE.
    Done,
    /// No supported operation could progress: the model answered BLOCKED,
    /// three steps changed nothing, DONE followed a covered attempt, the run
    /// went round in circles or never saw the page settle, the start page
    /// did not load, or a page was outside the allowed origins.
    Blocked,
    /// The action budget (60 by default, or `JEV_MAX_ACTIONS` and
    /// [`JevEngineConfig::with_max_actions`]) or the model-call budget of
    /// twice that ran out.
    BudgetExceeded,
    /// The run's timeout elapsed, during setup or the loop.
    TimedOut,
    /// A field needs a value nothing can supply: no text model, or the text
    /// model found none in the goal.
    NeedsInput,
    /// A model provider stayed unreachable or overloaded through its retries.
    Unavailable,
    /// Anything else that ended the run early.
    Error,
    /// The irreversible-action gate stopped the run before an action that
    /// may not be undone (see [`JevEngineConfig::with_irreversible_gate`]);
    /// `stopped_because` names the control. Nothing was dispatched.
    NeedsConfirmation,
    /// The site refused automated access: an HTTP 401, 403 or 429 page, a
    /// challenge or "unusual traffic" wall, or an "Access Denied" page with
    /// nothing to act on. Found on the first observation, it ends the run
    /// before any decision; `stopped_because` gives the evidence. Jev only
    /// reports it and never tries to get around it.
    AccessDenied,
}

impl JevStatus {
    pub(crate) fn stopped(self) -> bool {
        !matches!(self, Self::Ready)
    }
}

/// What ended a run that stopped short of its goal, beyond its status: a
/// `blocked` run may have stalled, found its target covered, been told
/// BLOCKED, or left the allowed origins, and a caller treats those
/// differently (Roder falls back to a model with full browser tools only
/// for some of them).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum JevStopCause {
    /// The model answered BLOCKED.
    ModelBlocked,
    /// A fresh UI observation failed the caller-defined completion predicates.
    OutcomeMismatch,
    /// Three actions in a row changed nothing.
    Stalled,
    /// Three actions in a row found their target covered.
    Covered,
    /// The model answered DONE straight after a covered attempt.
    DoneAfterCovered,
    /// A page was outside the allowed origins.
    OutsideScope,
    /// The start page did not load.
    NotLoaded,
    /// The action or model-call budget ran out.
    Budget,
    /// A provider, resolver or embedder ended the run with its own
    /// [`JevStop`], or an error did.
    Stopped,
    /// The decision service's reply could not be used three times running
    /// (the first answer and two asks again); the status is `error` and
    /// `stopped_because` gives the count and the first reason. Roder does
    /// not fall back after it.
    DecisionUnusable,
    /// The run was going round in circles: Jev was about to choose the same
    /// control a fourth time on a page that looked the same each time, or
    /// had waited six times in a row with nothing changing.
    /// `stopped_because` names the control.
    Looped,
    /// The page kept changing under Jev's decisions: three in a row went
    /// stale, each time with nothing new on the page. `stopped_because` gives
    /// the last stale message.
    Unsettled,
}

/// Ends a run with a typed status. The loop maps any other error to
/// [`JevStatus::Error`]; hosted decision clients, transports and text
/// resolvers can return this to say why they gave up.
#[derive(Debug)]
pub struct JevStop {
    status: JevStatus,
    reason: String,
}

impl JevStop {
    pub fn new(status: JevStatus, reason: impl Into<String>) -> Self {
        Self {
            status,
            reason: reason.into(),
        }
    }

    pub fn status(&self) -> JevStatus {
        self.status
    }

    /// The status a failed run ends with: this stop's, or `Error` for any
    /// other failure. A stop cannot claim the run is done or still going.
    pub(crate) fn status_of(error: &anyhow::Error) -> JevStatus {
        // Through any wrapper, such as a billed call's.
        let stop = error.chain().find_map(|cause| cause.downcast_ref::<Self>());
        match stop.map(Self::status) {
            Some(JevStatus::Ready | JevStatus::Done) | None => JevStatus::Error,
            Some(status) => status,
        }
    }
}

impl std::fmt::Display for JevStop {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.reason)
    }
}

impl std::error::Error for JevStop {}

pub struct JevEngineConfig {
    pub(crate) goal: String,
    pub(crate) decision: Arc<dyn JevDecisionClient>,
    pub(crate) text: Option<Arc<dyn JevTextValueResolver>>,
    pub(crate) wait: Duration,
    /// Executed actions a run may take; twice this many model calls.
    pub(crate) max_actions: usize,
    /// The longest a run may take, whatever `JevEngine::run` is given.
    pub(crate) max_duration: Option<Duration>,
    /// The origins the run may visit, checked after every observation.
    pub(crate) scope: JevOriginScope,
    /// Ask about, and stop before, actions that cannot be undone.
    pub(crate) irreversible_gate: bool,
    /// The gate lets a confident irreversible action through.
    pub(crate) irreversible_authorized: bool,
    /// Refuse a cookie banner once per document before reading it; on by
    /// default.
    pub(crate) refuse_cookie_banners: bool,
    /// Secrets typed by earlier runs on the same tab, scrubbed from this
    /// run's page reads too.
    pub(crate) secrets: Secrets,
}

impl JevEngineConfig {
    pub fn new(goal: impl Into<String>, decision: Arc<dyn JevDecisionClient>) -> Self {
        Self {
            goal: goal.into(),
            decision,
            text: None,
            wait: Duration::from_millis(800),
            max_actions: MAX_STEPS,
            max_duration: None,
            scope: JevOriginScope::any(),
            irreversible_gate: false,
            irreversible_authorized: false,
            refuse_cookie_banners: true,
            secrets: Secrets::default(),
        }
    }

    /// Start with the secrets an earlier run on this tab typed, so they stay
    /// scrubbed; the result's `typed_secrets` hands on the grown list.
    pub(crate) fn with_secrets(mut self, secrets: Secrets) -> Self {
        self.secrets = secrets;
        self
    }

    /// Cap the run at `actions` executed actions (at least one) and twice as
    /// many model calls; upstream's 60 by default.
    pub fn with_max_actions(mut self, actions: usize) -> Self {
        self.max_actions = actions.max(1);
        self
    }

    /// Cap the run's time, below any timeout `JevEngine::run` is given.
    pub fn with_max_duration(mut self, duration: Duration) -> Self {
        self.max_duration = Some(duration);
        self
    }

    /// Stop the run, `blocked`, once it observes a page outside `scope`.
    pub fn with_scope(mut self, scope: JevOriginScope) -> Self {
        self.scope = scope;
        self
    }

    /// Turn on the irreversible-action gate; off by default. Each decision
    /// request then also asks, in the same request, whether the offered
    /// clicks that may commit something, and any Enter, would make a
    /// purchase, payment, send, publish, delete or other change that cannot
    /// be undone. When the chosen action is one of those and the answer is
    /// above 0.5, or missing, the run ends [`JevStatus::NeedsConfirmation`]
    /// without dispatching it, unless it is authorized
    /// ([`with_irreversible_authorized`](Self::with_irreversible_authorized)).
    /// The decision client must implement
    /// [`JevDecisionClient::choose_gated`]. The thresholds are fastbrowse's
    /// and have not been validated against the hosted model.
    pub fn with_irreversible_gate(mut self) -> Self {
        self.irreversible_gate = true;
        self
    }

    /// Let the gate dispatch an irreversible action when the decision's
    /// call confidence is at least 0.90; below that the run still ends
    /// [`JevStatus::NeedsConfirmation`]. Only meaningful with the gate on.
    pub fn with_irreversible_authorized(mut self) -> Self {
        self.irreversible_authorized = true;
        self
    }

    /// Refuse cookie and consent banners, `on` by default. Before every
    /// observation the browser's [`JevBrowser::refuse_cookie_banner`] gets
    /// its chance, and the loop records a refusal as a step ("refused
    /// cookie banner: <label>") that costs no model call. Jev's own Chrome
    /// runner also injects DuckDuckGo's autoconsent into every document
    /// when this is on.
    pub fn with_cookie_banner_refusal(mut self, on: bool) -> Self {
        self.refuse_cookie_banners = on;
        self
    }

    pub fn with_text_resolver(mut self, text: Arc<dyn JevTextValueResolver>) -> Self {
        self.text = Some(text);
        self
    }

    pub fn with_wait(mut self, wait: Duration) -> Self {
        self.wait = wait;
        self
    }
}

/// Why a run whose timeout elapsed in the loop stopped.
pub(crate) const TIMED_OUT: &str = "Jev browser task timed out";

/// The reusable, bounded JEV agent loop.
pub struct JevEngine {
    agent: Agent,
}

impl JevEngine {
    pub async fn start(
        browser: Box<dyn JevBrowser>,
        config: JevEngineConfig,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            agent: Agent::start(browser, config).await?,
        })
    }

    pub async fn run(&mut self, timeout: Duration) -> JevRunResult {
        let timeout = self
            .agent
            .max_duration()
            .map_or(timeout, |max| timeout.min(max));
        // Model calls read the deadline, so a retry never outlasts the run.
        let deadline = tokio::time::Instant::now() + timeout;
        let run = tokio::time::timeout_at(deadline, self.agent.run());
        let stopped_because = match crate::http::RUN_DEADLINE.scope(deadline, run).await {
            Ok(stopped) => stopped,
            Err(_) => {
                self.agent.stop(JevStatus::TimedOut);
                Some(TIMED_OUT.into())
            }
        };
        // A timed-out run has spent its time; the result goes without facts.
        let stopped_because = self.agent.finish(stopped_because).await;
        self.agent.result(stopped_because)
    }

    pub async fn close(&mut self) -> anyhow::Result<()> {
        self.agent.close().await
    }
}

mod covered;
mod records;
