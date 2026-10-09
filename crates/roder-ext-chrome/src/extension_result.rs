//! Running a command on the paired extension, and reading what it answers.
//!
//! A command that shows the page (a snapshot, a navigation, a click, a typed
//! text, a key, a scroll, a choice in a select) is not handed to the model as
//! the extension's JSON. It is read into an [`Observed`] and rendered by
//! [`crate::observed_render`]: for an action, what it did to the page first,
//! then the controls with their refs, then the text, within one output
//! budget (characters and lines).
//! What an action did is found by comparing the page it shows with the last
//! page seen on the same tab; the first one has nothing to be compared with,
//! and says so.
//!
//! An extension built for `action-observation` answers an action with the
//! page it left: `{action, observation, tabId}`. An older build answers with
//! only what it did (`{ok, ref}`). For that build one `page/snapshot` is
//! asked for after a short wait, so the page has acted on the input, and its
//! answer stands in for the observation. Which of the two builds answered is
//! kept in the result's data, `observation.build`, and is never put in the
//! text the model reads: the model should act on the page, not on the
//! extension's age.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::Duration;

use roder_api::chrome::{ChromeCommand, ChromeController, ChromeError};
use serde_json::{Value, json};

use crate::observed::{Comparison, Observed, compare, one_line};
use crate::observed_render::{Budget, View, render};
use crate::session::{UNTRUSTED_NOTE, label_result, result_text};

/// How long the page of an action gets before an older build is asked what it
/// shows. The build before `action-observation` clicks 150 ms after it
/// answers, and this wait starts when the answer arrives, so the snapshot
/// reaches the page later than the click by about the round trip. That is a
/// thin margin: a page that reacts later than that (a request, an animation,
/// a form submit that navigates) can still be read mid-change.
pub(crate) const FALLBACK_DELAY: Duration = Duration::from_millis(150);
/// The sections of a snapshot the tools read.
const SNAPSHOT_INCLUDE: [&str; 4] = ["text", "controls", "forms", "iframes"];
/// Tabs whose last page is kept to compare the next one with.
const REMEMBERED_TABS: usize = 16;

/// The last page seen on each tab, and the wait for older builds.
pub(crate) struct ActionObserver {
    delay: Duration,
    seen: Mutex<Seen>,
}

/// The tab a call that no tab was named for is about.
const ACTIVE: &str = "active";

/// Which tab an answer is about.
#[derive(Clone, Copy)]
struct Tab {
    /// The tab the call named.
    called: Option<i64>,
    /// The tab the extension says it answered for (an action's answer does;
    /// a snapshot's does not).
    replied: Option<i64>,
}

impl Tab {
    fn of(params: &Value, reply: &Value) -> Self {
        Self {
            called: params["tabId"].as_i64(),
            replied: reply["tabId"].as_i64(),
        }
    }

    /// The tab, as far as anything said: the extension's word first.
    fn id(self) -> Option<i64> {
        self.replied.or(self.called)
    }
}

#[derive(Default)]
struct Seen {
    /// Oldest first.
    order: VecDeque<String>,
    pages: HashMap<String, Observed>,
}

impl Seen {
    /// The last page of `tab`. A call that named no tab is about the active
    /// one, which is also kept as [`ACTIVE`]; a page kept there is taken for a
    /// tab the extension names only while it is not known to be another's.
    /// A call that named a tab is compared with that tab's page only: a page
    /// of no known tab may be another tab's.
    fn page(&self, tab: Tab) -> Option<&Observed> {
        let own = tab.id().and_then(|id| self.pages.get(&id.to_string()));
        match (own, tab.called) {
            (Some(page), _) => Some(page),
            (None, Some(_)) => None,
            (None, None) => self.pages.get(ACTIVE).filter(|page| {
                tab.id()
                    .is_none_or(|id| page.tab_id.is_none_or(|was| was == id))
            }),
        }
    }
}

impl ActionObserver {
    pub(crate) fn new() -> Self {
        Self {
            delay: FALLBACK_DELAY,
            seen: Mutex::new(Seen::default()),
        }
    }

    fn baseline(&self, tab: Tab) -> Option<Observed> {
        self.seen.lock().ok()?.page(tab).cloned()
    }

    /// Keep `page` to compare the next one with. A page that did not read
    /// every section is filled in from the page before it, when that was the
    /// same page, so it does not replace a fuller one with a blank.
    fn remember(&self, tab: Tab, page: Observed) {
        let Ok(mut seen) = self.seen.lock() else {
            return;
        };
        let mut page = page.filled_from(seen.page(tab));
        page.tab_id = tab.id();
        let mut keys: Vec<String> = tab.id().map(|id| id.to_string()).into_iter().collect();
        if tab.called.is_none() {
            keys.push(ACTIVE.to_string());
        }
        for key in keys {
            if seen.pages.insert(key.clone(), page.clone()).is_none() {
                seen.order.push_back(key);
                while seen.order.len() > REMEMBERED_TABS {
                    if let Some(oldest) = seen.order.pop_front() {
                        seen.pages.remove(&oldest);
                    }
                }
            }
        }
    }

    /// The page after an action that came back without one: one snapshot,
    /// once the page has had time to act on the input.
    async fn read_after(
        &self,
        controller: &dyn ChromeController,
        params: &Value,
    ) -> Result<Value, String> {
        tokio::time::sleep(self.delay).await;
        let mut ask = json!({ "include": SNAPSHOT_INCLUDE });
        if let Some(tab) = params.get("tabId").filter(|tab| !tab.is_null()) {
            ask["tabId"] = tab.clone();
        }
        let reply = controller
            .dispatch(ChromeCommand::with_params("page/snapshot", ask))
            .await
            .map_err(|error| error.to_string())?;
        snapshot_of(&reply)
            .cloned()
            .ok_or_else(|| "the extension answered without a page".to_string())
    }
}

/// What a command's answer is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// Anything that is not a page: the extension's JSON, labelled.
    Plain,
    Snapshot,
    Navigation,
    /// An input to the page.
    Action,
}

fn shape(kind: &str) -> Shape {
    match kind {
        "page/snapshot" => Shape::Snapshot,
        "tab/navigate" => Shape::Navigation,
        "page/click" | "page/type" | "page/keypress" | "page/scroll" | "page/select" => {
            Shape::Action
        }
        _ => Shape::Plain,
    }
}

/// The snapshot in an answer: `{ok, snapshot}` from `page/snapshot`, an
/// observation itself, or a bare snapshot.
fn snapshot_of(value: &Value) -> Option<&Value> {
    let inner = value
        .get("snapshot")
        .filter(|snapshot| snapshot.is_object())
        .unwrap_or(value);
    ["url", "title", "text", "controls"]
        .iter()
        .any(|key| inner.get(*key).is_some())
        .then_some(inner)
}

/// What the action itself answered, apart from the page it left.
fn action_summary(value: &Value) -> Option<String> {
    let mut answer = match value.get("action").filter(|action| action.is_object()) {
        Some(action) => action.as_object()?.clone(),
        None => value.as_object()?.clone(),
    };
    for key in [
        "ok",
        "action",
        "observation",
        "snapshot",
        "tabId",
        "untrusted",
    ] {
        answer.remove(key);
    }
    (!answer.is_empty()).then(|| one_line(&Value::Object(answer).to_string(), 300))
}

/// Dispatch `kind` and shape what comes back into the result's data and the
/// text the model reads.
pub(crate) async fn run(
    controller: &dyn ChromeController,
    observer: &ActionObserver,
    kind: &str,
    params: &Value,
) -> Result<(Value, String), ChromeError> {
    let mut asked = params.clone();
    if kind == "page/snapshot"
        && (asked.is_object() || asked.is_null())
        && asked.get("include").is_none_or(Value::is_null)
    {
        // Everything the tools read: not the boxes, which they never use.
        asked["include"] = json!(SNAPSHOT_INCLUDE);
    }
    // The sections a snapshot was asked for: the others come back empty.
    let included = asked.get("include").and_then(Value::as_array).cloned();
    let value = controller
        .dispatch(ChromeCommand::with_params(kind, asked))
        .await?;
    let shape = shape(kind);
    let replied = match shape {
        Shape::Plain => None,
        Shape::Snapshot => snapshot_of(&value),
        Shape::Navigation | Shape::Action => snapshot_of(&value["observation"]),
    }
    .cloned();
    if replied.is_none() && shape != Shape::Action {
        return Ok(plain(kind, value));
    }
    let build = match replied.is_some() {
        true => "action-observation",
        false => "legacy",
    };
    let (snapshot, source) = match replied {
        Some(snapshot) => (Ok(snapshot), "action_result"),
        None => match observer.read_after(controller, params).await {
            Ok(snapshot) => (Ok(snapshot), "snapshot_fallback"),
            Err(error) => (Err(error), "unavailable"),
        },
    };
    let mut data = label_result(kind, value.clone());
    let action = action_summary(&value).filter(|_| shape == Shape::Action);
    if shape == Shape::Action {
        data["observation"] = json!({"build": build, "source": source});
    }
    let snapshot = match snapshot {
        Ok(snapshot) => snapshot,
        Err(error) => {
            data["observation"]["error"] = json!(error);
            let mut text = format!(
                "{UNTRUSTED_NOTE}\nThe action ran, but the page could not be read afterwards \
                 ({}). Call chrome_page_snapshot to see it.",
                one_line(&error, 200)
            );
            if let Some(action) = action {
                text.push_str(&format!("\nAction result: {action}"));
            }
            return Ok((data, text));
        }
    };
    let mut page = Observed::from_snapshot(&snapshot);
    if let Some(sections) = included.as_deref().filter(|_| shape == Shape::Snapshot) {
        page = page.limited_to(sections);
    }
    let tab = Tab::of(params, &value);
    page.tab_id = tab.id();
    let before = observer.baseline(tab);
    let comparison: Option<Comparison> =
        (shape == Shape::Action).then(|| compare(before.as_ref(), &page));
    observer.remember(tab, page.clone());
    if source == "snapshot_fallback" {
        data["observed"] = snapshot;
    }
    if let Some(comparison) = &comparison {
        data["outcome"] = comparison.data();
    }
    let text = render(
        &View {
            note: UNTRUSTED_NOTE,
            comparison: comparison.as_ref(),
            notices: Vec::new(),
            action,
            page: &page,
        },
        Budget::RESULT,
    );
    Ok((data, text))
}

/// An answer that is not a page: the extension's JSON, labelled untrusted when
/// it carries page content, and cut at the output cap.
fn plain(kind: &str, value: Value) -> (Value, String) {
    let data = label_result(kind, value);
    let text = result_text(kind, &data);
    (data, text)
}
