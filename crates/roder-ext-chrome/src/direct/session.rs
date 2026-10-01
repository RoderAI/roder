//! One attached tab and the tools run on it.
//!
//! [`DirectSession::run`] runs one tool by its short name (`look`, `click`,
//! …) and returns a [`DirectStep`]: the text the model reads, the data a
//! host keeps, and whether the owner's rules stopped the run. After every
//! action the session lets the page settle, follows a tab the action opened
//! (the tab it drives from then on), and reads the page again, briefly, so
//! the result shows what changed; the owner's guard then checks the page's
//! origin and whether it refused automated access.

use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::{Value, json};

use super::client::{DirectDialog, DirectTab, TabClient, cut};
use super::guard::{DirectGuard, StopKind};
use super::look::{self, Detail};

/// How long a page may take to finish loading after an action.
const LOAD_WAIT: Duration = Duration::from_secs(8);
const LOAD_POLL: Duration = Duration::from_millis(100);
/// The pause after an input before the page is looked at again.
const AFTER_INPUT: Duration = Duration::from_millis(150);
/// Then the page must go this long without a DOM change, within the cap.
const QUIET_MS: u64 = 250;
const QUIET_CAP_MS: u64 = 2000;

/// Why a step stopped the run it belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DirectStop {
    pub kind: StopKind,
    pub reason: String,
}

/// A tab an action opened, which the session now drives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenedTab {
    pub target_id: String,
    pub opener: String,
}

/// What one tool call came to.
#[derive(Debug, Clone, Default)]
pub struct DirectStep {
    /// What the model reads.
    pub text: String,
    /// The same, structured, for a host to keep: the tool, what it acted
    /// on, the page after it. Never a secret field's value or a typed
    /// secret.
    pub data: Value,
    pub is_error: bool,
    /// The owner's rules stopped the run here.
    pub stop: Option<DirectStop>,
    /// A screenshot, as a `data:` URL.
    pub image: Option<String>,
    /// A value typed into a secret field, for the owner to scrub from what
    /// it reads later. It appears nowhere else.
    pub typed_secret: Option<String>,
    /// A tab the action opened, now the one driven.
    pub opened_tab: Option<OpenedTab>,
}

impl DirectStep {
    pub fn error(message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            data: json!({"error": message}),
            text: message,
            is_error: true,
            ..Self::default()
        }
    }

    fn stopped(kind: StopKind, reason: String) -> Self {
        Self {
            data: json!({"stop": {"kind": kind, "reason": reason}}),
            text: reason.clone(),
            is_error: true,
            stop: Some(DirectStop { kind, reason }),
            ..Self::default()
        }
    }
}

/// One tab, attached and driven by the direct tools.
pub struct DirectSession {
    pub(crate) client: TabClient,
    pub(crate) guard: Arc<dyn DirectGuard>,
    /// A call's `authorize_irreversible` counts: the host asked the user.
    pub(crate) may_authorize: bool,
    /// Page targets open when the session attached, or since followed;
    /// a tab one of them opens later is the action's.
    known: Vec<String>,
}

impl DirectSession {
    /// Attach to `tab` under `guard`. With `may_authorize`, a call that sets
    /// `authorize_irreversible` passes the guard's confirmation check; the
    /// host must have asked the user for that call.
    pub async fn attach(
        tab: &DirectTab,
        guard: Arc<dyn DirectGuard>,
        may_authorize: bool,
    ) -> anyhow::Result<Self> {
        let mut client = TabClient::attach(tab).await?;
        let known = match client.browser_level() {
            true => page_targets(&mut client).await.unwrap_or_default(),
            false => Vec::new(),
        };
        Ok(Self {
            client,
            guard,
            may_authorize,
            known,
        })
    }

    /// Whether a call's `authorize_irreversible` counts on this session.
    pub fn may_authorize(&self) -> bool {
        self.may_authorize
    }

    /// The target the session drives now.
    pub fn target_id(&self) -> &str {
        self.client.target_id()
    }

    /// Run the tool `name` (a short name from [`super::DIRECT_TOOLS`]).
    pub async fn run(&mut self, name: &str, args: &Value) -> DirectStep {
        if let Err(error) = self.client.wait_cleanup().await {
            return DirectStep::error(format!("Browser cleanup prevented {name}: {error:#}"));
        }
        // Own cleanup outside the borrowed dispatch future: aborting a run
        // must release input even when its owner retains this session.
        let cleanup = self.client.cleanup();
        let mut step = match self.dispatch(name, args).await {
            Ok(step) => step,
            Err(error) => DirectStep::error(format!("{name} failed: {error:#}")),
        };
        if let Err(error) = cleanup.finish().await {
            step.is_error = true;
            step.text = format!("{error:#}\n{}", step.text);
            step.data["cleanup_error"] = json!(format!("{error:#}"));
        }
        step.data["tool"] = json!(name);
        step
    }

    async fn dispatch(&mut self, name: &str, args: &Value) -> anyhow::Result<DirectStep> {
        // Recheck on every call, including after external navigation or a stop
        // the caller ignored. Navigation to an allowed URL remains a recovery.
        if name != "navigate" {
            let current = self.client.evaluate_isolated("location.href").await?;
            if let Some(reason) = current.as_str().and_then(|url| self.guard.outside(url)) {
                return Ok(DirectStep::stopped(StopKind::OutsideScope, reason));
            }
        }
        match name {
            "look" => {
                let look = look::read(&mut self.client, self.guard.as_ref(), Detail::FULL).await?;
                let dialogs = self.client.take_dialogs();
                let mut text = look::render(&look);
                prepend_dialogs(&mut text, &dialogs);
                Ok(DirectStep {
                    text,
                    data: json!({"page": look, "dialogs": dialogs}),
                    ..DirectStep::default()
                })
            }
            "screenshot" => self.screenshot().await,
            "wait" => {
                let ms = args["ms"]
                    .as_u64()
                    .filter(|ms| *ms > 0)
                    .unwrap_or(1000)
                    .clamp(50, 10_000);
                tokio::time::sleep(Duration::from_millis(ms)).await;
                self.after(format!("Waited {ms} ms."), json!({"ms": ms}))
                    .await
            }
            "navigate" => self.navigate(args).await,
            _ => self.act(name, args).await,
        }
    }

    /// Settle after an input, follow a tab it opened, read the page again,
    /// and check it against the guard; `done` says what the action did.
    pub(crate) async fn after(
        &mut self,
        done: String,
        mut data: Value,
    ) -> anyhow::Result<DirectStep> {
        self.settle().await;
        let opened_tab = self.follow_opened_tab().await;
        if opened_tab.is_some() {
            self.settle().await;
        }
        let look = look::read(&mut self.client, self.guard.as_ref(), Detail::BRIEF).await?;
        let dialogs = self.client.take_dialogs();
        let facts = look::facts(&look);
        let mut text = done;
        if opened_tab.is_some() {
            text.push_str(&format!(
                " It opened a new tab, which these tools now drive ({}).",
                cut(&facts.url, 120)
            ));
        }
        prepend_dialogs(&mut text, &dialogs);
        data["page"] = look.clone();
        data["dialogs"] = json!(dialogs);
        if let Some(opened) = &opened_tab {
            data["opened_tab"] = json!(opened);
        }
        let stop = self
            .guard
            .outside(&facts.url)
            .map(|reason| (StopKind::OutsideScope, reason))
            .or_else(|| {
                self.guard
                    .refused(&facts)
                    .map(|reason| (StopKind::AccessDenied, reason))
            });
        if let Some((kind, reason)) = stop {
            let mut step = DirectStep::stopped(kind, format!("{text}\n{reason}"));
            step.data = json!({"stop": {"kind": kind, "reason": reason}, "page": look});
            step.opened_tab = opened_tab;
            return Ok(step);
        }
        Ok(DirectStep {
            text: format!("{text}\n{}", look::render(&look)),
            data,
            opened_tab,
            ..DirectStep::default()
        })
    }

    /// A step that stopped before anything was dispatched.
    pub(crate) fn confirm_first(&self, reason: String) -> DirectStep {
        DirectStep::stopped(StopKind::NeedsConfirmation, reason)
    }

    /// Wait out an input's effect: a short pause, a load it started, then
    /// a quiet document (at most [`QUIET_CAP_MS`]).
    async fn settle(&mut self) {
        tokio::time::sleep(AFTER_INPUT).await;
        let deadline = tokio::time::Instant::now() + LOAD_WAIT;
        while tokio::time::Instant::now() < deadline {
            match self.client.evaluate_isolated("document.readyState").await {
                Ok(state) if state != "loading" => break,
                _ => tokio::time::sleep(LOAD_POLL).await,
            }
        }
        // Best effort: a document that goes away meanwhile is read as it is.
        let _ = look::helper(
            &mut self.client,
            &format!("quiet({QUIET_MS}, {QUIET_CAP_MS})"),
        )
        .await;
    }

    /// A page the driven tab opened since the last look: attach to the
    /// newest one and drive it from now on.
    async fn follow_opened_tab(&mut self) -> Option<OpenedTab> {
        if !self.client.browser_level() {
            return None;
        }
        let targets = self
            .client
            .browser_call("Target.getTargets", json!({}))
            .await
            .ok()?;
        let opener = self.client.target_id().to_string();
        let opened = targets["targetInfos"]
            .as_array()?
            .iter()
            .filter(|target| target["type"] == "page")
            .filter(|target| target["openerId"].as_str() == Some(opener.as_str()))
            .filter_map(|target| target["targetId"].as_str())
            .filter(|id| !self.known.iter().any(|known| known == id))
            .map(str::to_string)
            .collect::<Vec<_>>();
        self.known.extend(opened.iter().cloned());
        let newest = opened.last()?.clone();
        self.client.attach_to(&newest).await.ok()?;
        Some(OpenedTab {
            target_id: newest,
            opener,
        })
    }

    /// Detach, leaving the tab open for its owner.
    pub async fn detach(self) {
        drop(self.client);
    }
}

/// Answered dialogs go first: they happened before the page as read.
fn prepend_dialogs(text: &mut String, dialogs: &[DirectDialog]) {
    if dialogs.is_empty() {
        return;
    }
    let lines = dialogs
        .iter()
        .map(|dialog| {
            format!(
                "[{} dialog, {}] {}",
                dialog.kind,
                if dialog.accepted {
                    "accepted"
                } else {
                    "dismissed"
                },
                cut(&dialog.message, 200)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    *text = format!("{lines}\n{text}");
}

async fn page_targets(client: &mut TabClient) -> anyhow::Result<Vec<String>> {
    let targets = client.browser_call("Target.getTargets", json!({})).await?;
    Ok(targets["targetInfos"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|target| target["type"] == "page")
        .filter_map(|target| target["targetId"].as_str().map(str::to_string))
        .collect())
}
