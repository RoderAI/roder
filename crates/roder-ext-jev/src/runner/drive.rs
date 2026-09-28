//! One task on a Chrome endpoint, under one deadline.
//!
//! Upstream's timeout covers only the agent loop, so a task could spend 20 s
//! starting Chrome and 15 s loading its page before its own clock started.
//! Here every phase, from connecting to the last step, runs against the same
//! deadline, and a phase that runs out ends the task as `timed_out` naming
//! that phase. The tab is closed on every failure after it was created,
//! whether the phase failed or ran out of time.

use std::time::Duration;

use tokio::time::Instant;

use crate::cdp::Connection;
use crate::engine::{JevEngine, JevEngineConfig, JevRunResult, JevStatus};
use crate::page::{self, LoadFailed, Page};

/// How long closing an abandoned tab may take, past the deadline.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(3);

pub(crate) struct Task {
    pub(crate) url: String,
    pub(crate) foreground: bool,
    /// When the task started, for its reported elapsed time.
    pub(crate) started: Instant,
    pub(crate) deadline: Instant,
    pub(crate) config: JevEngineConfig,
}

/// Why a task whose deadline passed during setup stopped.
pub(crate) fn timed_out(phase: &str) -> String {
    format!("Jev browser task timed out while {phase}")
}

/// `Some` with the phase's own result, or `None` when the deadline passed.
async fn within<T>(deadline: Instant, phase: impl Future<Output = T>) -> Option<T> {
    tokio::time::timeout_at(deadline, phase).await.ok()
}

/// Open the start page on `endpoint` and run the loop until the deadline. A
/// page that does not load ends `blocked` without starting the loop.
pub(crate) async fn drive(endpoint: &str, task: Task) -> anyhow::Result<JevRunResult> {
    let Task {
        url,
        foreground,
        started,
        deadline,
        config,
    } = task;
    let stop = |status, reason: String| {
        Ok(JevRunResult::before_start(
            status,
            &url,
            started.elapsed(),
            reason,
        ))
    };

    let Some(connection) = within(deadline, Connection::connect(endpoint)).await else {
        return stop(JevStatus::TimedOut, timed_out("connecting to Chrome"));
    };
    let mut connection = connection?;
    // Only `Target.createTarget` itself can be cut off with a tab left open,
    // and Chrome answers it at once.
    let Some(target) = within(deadline, page::create_target(&mut connection)).await else {
        return stop(JevStatus::TimedOut, timed_out("opening a tab"));
    };
    let target = target?;
    // Attaching (and showing a foreground tab, before it loads, so the work
    // is watchable and the final page stays on screen) can be cut off with
    // the tab open, so a cut-off closes it by id.
    let Some(page) = within(
        deadline,
        Page::attach(connection, target.clone(), foreground),
    )
    .await
    else {
        close_by_id(endpoint, &target).await;
        return stop(JevStatus::TimedOut, timed_out("opening a tab"));
    };
    let mut page = page?;
    page.inject_autoconsent(config.refuse_cookie_banners);
    match within(deadline, page.load(&url)).await {
        Some(Ok(())) => {}
        Some(Err(error)) => {
            close_page(&mut page).await;
            return match error.downcast::<LoadFailed>() {
                Ok(failed) => stop(JevStatus::Blocked, failed.to_string()),
                Err(error) => Err(error),
            };
        }
        None => {
            close_page(&mut page).await;
            return stop(JevStatus::TimedOut, timed_out("loading the page"));
        }
    }

    // The engine owns the page from here, and a start that fails or runs
    // out of time drops it, so the tab is closed by id on a new connection.
    let target = page.target_id().to_string();
    let mut engine = match within(deadline, JevEngine::start(Box::new(page), config)).await {
        Some(Ok(engine)) => engine,
        Some(Err(error)) => {
            close_by_id(endpoint, &target).await;
            return Err(error);
        }
        None => {
            close_by_id(endpoint, &target).await;
            return stop(JevStatus::TimedOut, timed_out("observing the page"));
        }
    };
    let result = engine
        .run(deadline.saturating_duration_since(Instant::now()))
        .await;
    if !foreground {
        close_quietly(engine.close()).await;
    }
    Ok(result)
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
async fn close_quietly(close: impl Future<Output = anyhow::Result<()>>) {
    let _ = tokio::time::timeout(CLOSE_TIMEOUT, close).await;
}
