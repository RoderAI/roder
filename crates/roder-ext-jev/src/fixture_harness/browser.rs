//! The throwaway headless Chrome the fixture tests share.
//!
//! One Chrome serves the whole test process, started on the first use by
//! whichever test comes first and kept for all the rest ([`TestChrome::launch`]).
//! A Chrome per test, as there used to be, meant one per test thread: a
//! full run of the crate started up to a dozen at once, each with the
//! renderer, GPU and network processes of a browser, and exhausted the memory
//! of a developer's Mac.
//!
//! What keeps tests apart on it:
//!
//! - each test is handed a [`TabRelay`] as its Chrome's DevTools address, and
//!   with it a browser context of its own: a window of its own, so that
//!   bringing a tab to the front or moving focus does not take either from
//!   another test's page; storage of its own, so that no cookie or
//!   localStorage crosses tests; and a set of tabs of its own, which
//!   [`TestChrome::owned_pages`] lists, so that a test counts and checks only
//!   its tabs. The context is disposed of when the test ends, closing the
//!   tabs it left open;
//! - each test serves its fixtures from its own `127.0.0.1:0` port, so even
//!   an origin is the test's own;
//! - a test must not do what ends or changes the browser for every other:
//!   `Browser.close`, crashing it, or browser-wide settings. One that has to
//!   needs a Chrome of its own. Today the only private launcher is the
//!   windowed [`TestChrome::launch_headed`]; the first test that needs a
//!   headless one adds it. Jev's own launch tests start Chromes through
//!   `chrome::launch_with` on profiles of their own.
//!
//! The Chrome starts under a watcher that ends it when the test process
//! does, however that happens (see [`chrome_process`](super::chrome_process)).
//! A Chrome that has stopped answering is replaced by the next test to
//! start, and a Chrome that cannot be started is reported to every test, not
//! only the first.
//!
//! Whether a missing Chrome skips or fails is decided here too (see
//! [`binaries`]).

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Once, PoisonError};
use std::time::Duration;

use anyhow::Context;
use serde_json::Value;

use super::chrome_process::ChromeProcess;
use super::tab_relay::TabRelay;
use crate::chrome::chrome_candidates;

/// A test's handle on a Chrome: the Chrome, and the test's own DevTools
/// address on it. Dropping it closes the tabs the test left open, and a
/// Chrome nobody shares.
pub(crate) struct TestChrome {
    // In drop order: the tabs are closed while the Chrome is still there.
    relay: TabRelay,
    _process: Arc<ChromeProcess>,
    pub(crate) endpoint: String,
}

impl TestChrome {
    /// A handle on the process-wide Chrome, started if no test has yet.
    /// `Ok(None)` when no Chrome binary exists on this machine and none is
    /// required, so callers can skip; see [`binaries`]. A Chrome that starts
    /// but never answers is an error, for this test and every later one.
    pub(crate) async fn launch() -> anyhow::Result<Option<Self>> {
        let process = tokio::task::spawn_blocking(shared_chrome)
            .await
            .context("start the shared test Chrome")??;
        match process {
            Some(process) => Self::attach(process, true).await.map(Some),
            None => Ok(None),
        }
    }

    /// A Chrome of its own with a real window, for measuring what a
    /// background tab of a visible browser does; headless Chrome treats
    /// every tab as shown. Closed when the handle is dropped.
    pub(crate) async fn launch_headed() -> anyhow::Result<Option<Self>> {
        let process = tokio::task::spawn_blocking(|| {
            binaries()?
                .map(|binaries| ChromeProcess::start(&binaries, false).map(Arc::new))
                .transpose()
        })
        .await
        .context("start a windowed test Chrome")??;
        match process {
            Some(process) => Self::attach(process, false).await.map(Some),
            None => Ok(None),
        }
    }

    async fn attach(process: Arc<ChromeProcess>, shared: bool) -> anyhow::Result<Self> {
        let relay = TabRelay::start(process.clone(), shared).await?;
        Ok(Self {
            endpoint: relay.url().to_string(),
            relay,
            _process: process,
        })
    }

    /// The page tabs this test owns that Chrome lists now: those in its
    /// browser context.
    pub(crate) async fn owned_pages(&self) -> anyhow::Result<Vec<Value>> {
        self.relay.owned_pages().await
    }
}

/// The Chrome of this test process, started on the first call. Blocking:
/// run it on a blocking thread, as nothing about the Chrome may belong to
/// the runtime of the test that happens to start it.
fn shared_chrome() -> anyhow::Result<Option<Arc<ChromeProcess>>> {
    static SHARED: Slot<ChromeProcess> = Slot::new();
    let Some(binaries) = binaries()? else {
        return Ok(None);
    };
    SHARED
        .get(ChromeProcess::answers, || {
            static STARTED: AtomicBool = AtomicBool::new(false);
            if STARTED.swap(true, Ordering::SeqCst) {
                // Written past the test harness's capture, like the note in
                // `binaries`: the tests that were using the old one fail.
                let _ = writeln!(
                    std::io::stderr(),
                    "roder-ext-jev: the shared test Chrome stopped answering (crashed, or \
                     killed from outside); starting another. Tests that were running on it \
                     fail"
                );
            }
            ChromeProcess::start(&binaries, true)
        })
        .map(Some)
}

/// A value started once, shared by everyone who asks, and started again only
/// when it has gone. A failure to start is kept and given to every caller
/// after it, instead of every test waiting out its own failed start.
struct Slot<T> {
    value: Mutex<Option<Result<Arc<T>, String>>>,
}

impl<T> Slot<T> {
    const fn new() -> Self {
        Self {
            value: Mutex::new(None),
        }
    }

    /// The value, started by `start` when there is none or `live` says the
    /// one there is has gone. Callers wait while one starts it.
    fn get(
        &self,
        live: impl Fn(&T) -> bool,
        start: impl FnOnce() -> anyhow::Result<T>,
    ) -> anyhow::Result<Arc<T>> {
        let mut value = self.value.lock().unwrap_or_else(PoisonError::into_inner);
        match value.as_ref() {
            Some(Ok(started)) if live(started) => return Ok(started.clone()),
            Some(Err(failure)) => anyhow::bail!("{failure}"),
            _ => {}
        }
        let started = start().map(Arc::new).map_err(|error| format!("{error:#}"));
        *value = Some(started.clone());
        started.map_err(|failure| anyhow::anyhow!(failure))
    }
}

/// Whether a missing Chrome fails the run: `JEV_REQUIRE_CHROME=1`, or `CI`
/// set as CI services set it.
fn chrome_required() -> bool {
    let set = |name: &str| {
        std::env::var(name).is_ok_and(|value| {
            !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "" | "0" | "false" | "no"
            )
        })
    };
    set("JEV_REQUIRE_CHROME") || set("CI")
}

/// The Chrome binaries to try: `JEV_CHROME_BINARY` alone when it is set,
/// which must then be an executable file, else the platform's usual ones
/// that exist. `Ok(None)` when there are none and none is required, after
/// saying once, on the terminal, that Chrome-backed tests are skipped;
/// when one is required, that is an error.
pub(crate) fn binaries() -> anyhow::Result<Option<Vec<String>>> {
    let explicit = std::env::var("JEV_CHROME_BINARY").ok();
    let found = chrome_candidates(None)
        .into_iter()
        .filter(|binary| resolve(binary).is_some())
        .collect::<Vec<_>>();
    let chosen = choose(explicit.as_deref(), found, chrome_required())?;
    if chosen.is_none() {
        static NOTE: Once = Once::new();
        NOTE.call_once(|| {
            // Written past the test harness's capture, so it shows without
            // --nocapture: a skipped test otherwise reads as a pass.
            let _ = writeln!(
                std::io::stderr(),
                "roder-ext-jev: no Chrome binary found, so every Chrome-backed test in this run \
                 is SKIPPED and reported as passing; set JEV_CHROME_BINARY, or \
                 JEV_REQUIRE_CHROME=1 to fail instead"
            );
        });
    }
    Ok(chosen)
}

/// The policy behind [`binaries`]: an explicit binary must be executable; with
/// none found, a required Chrome is an error and an optional one `None`.
fn choose(
    explicit: Option<&str>,
    found: Vec<String>,
    required: bool,
) -> anyhow::Result<Option<Vec<String>>> {
    if let Some(explicit) = explicit.map(str::trim).filter(|value| !value.is_empty()) {
        anyhow::ensure!(
            executable(Path::new(explicit)),
            "JEV_CHROME_BINARY is set to {explicit:?}, which is not an executable file"
        );
        return Ok(Some(vec![explicit.to_string()]));
    }
    if !found.is_empty() {
        return Ok(Some(found));
    }
    anyhow::ensure!(
        !required,
        "no Chrome binary found, and JEV_REQUIRE_CHROME or CI requires the Chrome-backed tests \
         to run; install Chrome or set JEV_CHROME_BINARY"
    );
    Ok(None)
}

/// A binary name as the OS would run it: a path that exists, or a name
/// found on `PATH`.
fn resolve(binary: &str) -> Option<PathBuf> {
    let path = Path::new(binary);
    if path.components().count() > 1 {
        return executable(path).then(|| path.to_path_buf());
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(binary))
        .find(|candidate| executable(candidate))
}

fn executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::metadata(path)
            .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn a_failed_start_is_given_to_every_caller_and_not_tried_again() {
        let slot = Slot::<u32>::new();
        let starts = Cell::new(0);
        let start = || {
            starts.set(starts.get() + 1);
            Err(anyhow::anyhow!("Chrome did not report a port"))
        };
        for _ in 0..3 {
            let error = slot.get(|_| true, start).unwrap_err().to_string();
            assert!(error.contains("did not report a port"), "{error}");
        }
        assert_eq!(starts.get(), 1);
    }

    #[test]
    fn one_start_serves_everyone_until_the_value_has_gone() {
        let slot = Slot::new();
        let first = slot.get(|_| true, || Ok(1u32)).unwrap();
        let again = slot.get(|_| true, || Ok(2)).unwrap();
        assert!(Arc::ptr_eq(&first, &again));
        assert_eq!(*again, 1);

        // Gone: the next caller starts a new one, and the rest share it.
        let replaced = slot.get(|value| *value != 1, || Ok(3)).unwrap();
        assert_eq!(*replaced, 3);
        assert_eq!(*slot.get(|_| true, || Ok(4)).unwrap(), 3);
    }

    #[test]
    fn callers_that_arrive_together_wait_for_the_one_start() {
        let slot = Arc::new(Slot::new());
        let starts = Arc::new(AtomicUsize::new(0));
        let callers = (0..8)
            .map(|_| {
                let (slot, starts) = (slot.clone(), starts.clone());
                std::thread::spawn(move || {
                    slot.get(
                        |_| true,
                        || {
                            starts.fetch_add(1, Ordering::SeqCst);
                            std::thread::sleep(Duration::from_millis(100));
                            Ok(7u32)
                        },
                    )
                    .unwrap()
                })
            })
            .collect::<Vec<_>>();
        let values = callers
            .into_iter()
            .map(|caller| caller.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(starts.load(Ordering::SeqCst), 1);
        assert!(values.iter().all(|value| Arc::ptr_eq(value, &values[0])));
    }

    #[test]
    fn a_missing_chrome_skips_only_when_none_is_required() {
        let found = vec!["/opt/chrome".to_string()];
        assert_eq!(choose(None, found.clone(), true).unwrap(), Some(found));
        assert_eq!(choose(None, Vec::new(), false).unwrap(), None);
        let error = choose(None, Vec::new(), true).unwrap_err().to_string();
        assert!(error.contains("JEV_REQUIRE_CHROME"), "{error}");
        // An explicit binary that cannot run is an error, not "no Chrome".
        let error = choose(Some("/no/such/chrome"), Vec::new(), false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("JEV_CHROME_BINARY"), "{error}");
        let error = choose(Some("/etc/hosts"), Vec::new(), false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("not an executable file"), "{error}");
        assert_eq!(
            choose(Some(" /bin/sh "), Vec::new(), false).unwrap(),
            Some(vec!["/bin/sh".to_string()])
        );
        assert_eq!(choose(Some("  "), Vec::new(), false).unwrap(), None);
    }
}
