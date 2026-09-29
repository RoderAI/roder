//! One task on a Chrome endpoint, in the session's tab, under one deadline.
//!
//! Upstream's timeout covers only the agent loop, so a task could spend 20 s
//! starting Chrome and 15 s loading its page before its own clock started.
//! Here every phase, from connecting to the last step, runs against the same
//! deadline, and a phase that runs out ends the task as `timed_out` naming
//! that phase.
//!
//! A call goes on in the tab the session's last call left, re-attached over
//! a new connection ([`Page::resume`]), and loads its url there only when the
//! tab is not already on it. It opens a tab only for the session's first
//! call, `tab: "new"`, a reset, or when the recorded tab is gone, and closes
//! only a tab it opened itself that failed before the loop started. When the
//! call ends, its tabs stay open for the next one; the session closes them.

use std::time::Duration;

use reqwest::Url;
use tokio::time::Instant;

use super::request::TabChoice;
use crate::cdp::Connection;
use crate::engine::{JevEngine, JevEngineConfig, JevRunResult, JevStatus};
use crate::page::{self, LoadFailed, Page, TabsGone};
use crate::session::{SessionTabs, TabNote};

/// How long closing an abandoned tab may take, past the deadline.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(3);

pub(crate) struct Task {
    /// The url to load, or `None` to go on from the page the tab shows.
    pub(crate) url: Option<String>,
    pub(crate) tab: TabChoice,
    /// The target of a tab whose page refused access, which a call that
    /// asked for a new tab loads its url in instead ([`TabChoice::Current`]).
    /// Should that tab be gone, the call opens the new tab it asked for
    /// rather than loading over the tab before it.
    pub(crate) refused_tab: Option<String>,
    pub(crate) foreground: bool,
    /// When the task started, for its reported elapsed time.
    pub(crate) started: Instant,
    pub(crate) deadline: Instant,
    pub(crate) config: JevEngineConfig,
}

/// A task's result and how it came to be on its tab.
#[derive(Debug)]
pub(crate) struct Driven {
    pub(crate) result: JevRunResult,
    pub(crate) note: TabNote,
    /// Where the tab was when this call found it, when that is not where
    /// the last call left it (the user moved it).
    pub(crate) moved_to: Option<String>,
}

/// Why a task whose deadline passed during setup stopped.
pub(crate) fn timed_out(phase: &str) -> String {
    format!("Jev browser task timed out while {phase}")
}

/// `Some` with the phase's own result, or `None` when the deadline passed.
pub(crate) async fn within<T>(deadline: Instant, phase: impl Future<Output = T>) -> Option<T> {
    tokio::time::timeout_at(deadline, phase).await.ok()
}

/// Whether two addresses name the same page, as a URL parser reads them.
pub(crate) fn same_page(a: &str, b: &str) -> bool {
    match (Url::parse(a.trim()), Url::parse(b.trim())) {
        (Ok(a), Ok(b)) => a == b,
        _ => a.trim() == b.trim(),
    }
}

/// Run `task` in the session's tabs on `endpoint` until its deadline, and
/// record the tabs it ends with in `tabs`. A page that does not load ends
/// `blocked` without starting the loop.
pub(crate) async fn drive(
    endpoint: &str,
    tabs: &mut SessionTabs,
    task: Task,
) -> anyhow::Result<Driven> {
    let Task {
        url,
        mut tab,
        refused_tab,
        foreground,
        started,
        deadline,
        config,
    } = task;
    let shown = url
        .clone()
        .or_else(|| tabs.last_url.clone())
        .unwrap_or_default();
    let mut note = TabNote::New;
    let stop = |status, reason: String, note: TabNote| {
        Ok(Driven {
            result: JevRunResult::before_start(status, &shown, started.elapsed(), reason),
            note,
            moved_to: None,
        })
    };

    let Some(connection) = within(deadline, Connection::connect(endpoint)).await else {
        return stop(JevStatus::TimedOut, timed_out("connecting to Chrome"), note);
    };
    let mut connection = Some(connection?);
    if tab == TabChoice::Reset
        && let Some(connection) = connection.as_mut()
    {
        close_all(connection, &tabs.targets()).await;
        tabs.clear();
    }

    // Go on in the recorded tabs, unless the call asked for a new one.
    let mut resumed = None;
    let mut moved_to = None;
    if tab == TabChoice::Current && !tabs.is_empty() {
        let owned = tabs.owned();
        let was = tabs.current().cloned();
        let taken = connection.take().expect("connected above");
        match within(
            deadline,
            Page::resume(taken, &owned, foreground, config.refuse_cookie_banners),
        )
        .await
        {
            None => {
                return stop(
                    JevStatus::TimedOut,
                    timed_out("returning to the tab"),
                    TabNote::Continued,
                );
            }
            Some(Ok((page, gone)))
                if refused_tab
                    .as_ref()
                    .is_some_and(|refused| refused != page.target_id()) =>
            {
                // The refused tab is gone: open the tab the call asked for,
                // keeping the page of the one before it. Dropping the page
                // leaves its tabs open.
                tabs.forget(&gone);
                drop(page);
                tab = TabChoice::New;
            }
            Some(Ok((page, gone))) => {
                tabs.forget(&gone);
                note = match was {
                    Some(was) if gone.contains(&was.target) => TabNote::Reopened(format!(
                        "{} was closed; went on in {}, the tab before it, without reloading",
                        was.id,
                        tabs.id_of(page.target_id()).unwrap_or("the tab before it"),
                    )),
                    _ => TabNote::Continued,
                };
                resumed = Some(page);
            }
            Some(Err(error)) if error.is::<TabsGone>() => {
                let ids = tabs
                    .records
                    .iter()
                    .map(|record| record.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                let reload = url.clone().or_else(|| tabs.last_url.clone());
                note = TabNote::Reopened(match &reload {
                    Some(reload) => format!(
                        "{ids} was closed; opened a new tab at {reload}, and anything \
                         entered there before was lost"
                    ),
                    None => format!("{ids} was closed; opened a new tab"),
                });
                let last_url = tabs.last_url.take();
                tabs.clear();
                tabs.last_url = last_url;
            }
            Some(Err(error)) => return Err(error),
        }
    }

    let beside = tab == TabChoice::New && !tabs.is_empty();
    // The tab this call opened, closed again if the task fails before its loop.
    let mut opened = None;
    let page = match resumed {
        Some(mut page) => {
            let Some(now_at) = within(deadline, page.current_url()).await else {
                return stop(JevStatus::TimedOut, timed_out("returning to the tab"), note);
            };
            // The recorded address has typed secrets scrubbed from it (a GET
            // form's address), so the live one is compared, and reported,
            // scrubbed the same way.
            let now_at = config.secrets.scrub(&now_at?);
            if let Some(last) = &tabs.last_url
                && note == TabNote::Continued
                && !same_page(last, &now_at)
            {
                moved_to = Some(now_at.clone());
            }
            if let Some(url) = url.as_deref().filter(|url| !same_page(url, &now_at)) {
                if note == TabNote::Continued {
                    note = TabNote::Navigated;
                }
                tabs.last_url = Some(url.to_string());
                match within(deadline, page.goto(url)).await {
                    Some(Ok(())) => {}
                    Some(Err(error)) => {
                        return match error.downcast::<LoadFailed>() {
                            Ok(failed) => stop(JevStatus::Blocked, failed.to_string(), note),
                            Err(error) => Err(error),
                        };
                    }
                    None => {
                        return stop(JevStatus::TimedOut, timed_out("loading the page"), note);
                    }
                }
            }
            page
        }
        None => {
            let Some(load) = url.clone().or_else(|| tabs.last_url.clone()) else {
                anyhow::bail!(super::NO_TAB_YET);
            };
            // A call's own url was checked against the operator's origins
            // when it was parsed; the page the last call ended on was not,
            // and may be one the run stopped at for being outside them.
            if !config.scope.allows(&load) {
                return stop(JevStatus::Blocked, config.scope.outside(&load), note);
            }
            // `resume` took the connection when the recorded tabs were gone.
            let mut connection = match connection.take() {
                Some(connection) => connection,
                None => match within(deadline, Connection::connect(endpoint)).await {
                    Some(connection) => connection?,
                    None => {
                        return stop(JevStatus::TimedOut, timed_out("connecting to Chrome"), note);
                    }
                },
            };
            // Only `Target.createTarget` itself can be cut off with a tab
            // left open, and Chrome answers it at once.
            let Some(target) = within(deadline, page::create_target(&mut connection)).await else {
                return stop(JevStatus::TimedOut, timed_out("opening a tab"), note);
            };
            let target = target?;
            // Attaching (and showing a foreground tab, before it loads, so
            // the work is watchable) can be cut off with the tab open, so a
            // cut-off closes it by id.
            let Some(page) = within(
                deadline,
                Page::attach(connection, target.clone(), foreground),
            )
            .await
            else {
                close_by_id(endpoint, &target).await;
                return stop(JevStatus::TimedOut, timed_out("opening a tab"), note);
            };
            let mut page = page?;
            page.inject_autoconsent(config.refuse_cookie_banners);
            match within(deadline, page.load(&load)).await {
                Some(Ok(())) => {}
                Some(Err(error)) => {
                    close_page(&mut page).await;
                    return match error.downcast::<LoadFailed>() {
                        Ok(failed) => stop(JevStatus::Blocked, failed.to_string(), note),
                        Err(error) => Err(error),
                    };
                }
                None => {
                    close_page(&mut page).await;
                    return stop(JevStatus::TimedOut, timed_out("loading the page"), note);
                }
            }
            opened = Some(target);
            page
        }
    };

    // The engine owns the page from here, and a start that fails or runs
    // out of time drops it, so a tab this call opened is closed by id on a
    // new connection. The ledger says which tabs the page ends with.
    let ledger = page.ledger();
    let mut engine = match within(deadline, JevEngine::start(Box::new(page), config)).await {
        Some(Ok(engine)) => engine,
        Some(Err(error)) => {
            if let Some(target) = &opened {
                close_by_id(endpoint, target).await;
            }
            return Err(error);
        }
        None => {
            if let Some(target) = &opened {
                close_by_id(endpoint, target).await;
            }
            return stop(JevStatus::TimedOut, timed_out("observing the page"), note);
        }
    };
    let result = engine
        .run(deadline.saturating_duration_since(Instant::now()))
        .await;
    // Dropping the engine closes its connection and leaves the tabs open.
    drop(engine);
    let ledger = ledger
        .lock()
        .map(|ledger| ledger.clone())
        .unwrap_or_default();
    let mut closing = tabs.absorb(&ledger, beside);
    closing.extend(tabs.cap());
    if !closing.is_empty() {
        close_quietly(async {
            let mut connection = Connection::connect(endpoint).await?;
            close_all(&mut connection, &closing).await;
            Ok(())
        })
        .await;
    }
    if !result.url.is_empty() {
        tabs.last_url = Some(result.url.clone());
    }
    Ok(Driven {
        result,
        note,
        moved_to,
    })
}

/// Close each of `targets`, bounded, ignoring failures: a tab already gone
/// is closed.
pub(crate) async fn close_all(connection: &mut Connection, targets: &[String]) {
    for target in targets {
        close_quietly(page::close_target(connection, target)).await;
    }
}

async fn close_page(page: &mut Page) {
    close_quietly(page.close()).await;
}

/// Close a tab this task can no longer reach through its page.
async fn close_by_id(endpoint: &str, target: &str) {
    close_quietly(async {
        let mut connection = Connection::connect(endpoint).await?;
        page::close_target(&mut connection, target).await
    })
    .await;
}

/// Cleanup after a failure: bounded, and never the error the task reports.
pub(crate) async fn close_quietly(close: impl Future<Output = anyhow::Result<()>>) {
    let _ = tokio::time::timeout(CLOSE_TIMEOUT, close).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_page_is_compared_as_parsed() {
        assert!(same_page("https://Example.com", "https://example.com/"));
        assert!(same_page(" https://a.test/x?q=1 ", "https://a.test/x?q=1"));
        assert!(!same_page("https://a.test/x", "https://a.test/y"));
        assert!(!same_page("https://a.test/#top", "https://a.test/"));
        assert!(same_page("about:blank", "about:blank"));
    }
}
