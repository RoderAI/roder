//! A throwaway headless Chrome for fixture tests.
//!
//! Each harness starts its own Chrome (headless, except for the one ignored
//! test that measures a real window) on a fresh temporary profile and lets
//! Chrome pick a free DevTools port (`--remote-debugging-port=0`, read back
//! from `DevToolsActivePort`). It never probes 9222 and never touches the
//! persistent `jev-chrome` profile.
//!
//! When dropped, including when a test panics, the Chrome is asked to close
//! (`Browser.close`), then killed if it has not exited, and its profile is
//! removed. A kill alone leaves the temporary directory Chrome keeps its
//! profile's singleton socket in (one `com.google.Chrome.*` per launch on
//! macOS, where Chrome ignores `TMPDIR`; the child's `TMPDIR` is its profile,
//! for the platforms that honour it). A profile left by a test process that
//! died is removed, and its Chrome stopped, by the next run's first launch.
//!
//! Whether a missing Chrome skips or fails is decided here too (see
//! [`binaries`]).

use std::io::Write as _;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Once;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use tokio_tungstenite::tungstenite::{self, Message};

use crate::chrome::{chrome_candidates, read_active_port};

/// Generous: with every fixture test starting its own Chrome at once on a
/// loaded machine, one took over 20 s to report its port.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(60);
/// How long a Chrome asked to close may take before it is killed.
const CLOSE_WAIT: Duration = Duration::from_secs(5);
/// Every profile a test Chrome runs on starts with this, then the test
/// process's id and a counter.
const PROFILE_PREFIX: &str = "roder-jev-test-";

pub(crate) struct TestChrome {
    child: Child,
    profile: PathBuf,
    pub(crate) endpoint: String,
}

impl TestChrome {
    /// `Ok(None)` when no Chrome binary exists on this machine and none is
    /// required, so callers can skip; see [`binaries`]. A Chrome that starts
    /// but never answers is an error.
    pub(crate) async fn launch() -> anyhow::Result<Option<Self>> {
        Self::launch_with(true).await
    }

    /// A Chrome with a real window, for measuring what a background tab of a
    /// visible browser does; headless Chrome treats every tab as shown.
    pub(crate) async fn launch_headed() -> anyhow::Result<Option<Self>> {
        Self::launch_with(false).await
    }

    async fn launch_with(headless: bool) -> anyhow::Result<Option<Self>> {
        let Some(binaries) = binaries()? else {
            return Ok(None);
        };
        reap_orphans();
        let mut failures = Vec::new();
        for binary in binaries {
            let profile = temp_profile();
            let child = match spawn(&binary, &profile, headless) {
                Ok(child) => child,
                Err(error) => {
                    let _ = std::fs::remove_dir_all(&profile);
                    failures.push(format!("{binary}: {error}"));
                    continue;
                }
            };
            // Owned from here on: an early return below still stops Chrome.
            let mut chrome = Self {
                child,
                profile,
                endpoint: String::new(),
            };
            std::fs::write(
                chrome.profile.join("chrome.pid"),
                chrome.child.id().to_string(),
            )
            .context("record the test Chrome's pid")?;
            let port = chrome.wait_for_port().await?;
            chrome.endpoint = format!("http://127.0.0.1:{port}");
            return Ok(Some(chrome));
        }
        bail!(
            "no Chrome binary could be started ({})",
            failures.join("; ")
        )
    }

    async fn wait_for_port(&mut self) -> anyhow::Result<u16> {
        let file = self.profile.join("DevToolsActivePort");
        let deadline = tokio::time::Instant::now() + STARTUP_TIMEOUT;
        while tokio::time::Instant::now() < deadline {
            if let Some(status) = self.child.try_wait()? {
                bail!("test Chrome exited during startup: {status}");
            }
            if let Some(port) = read_active_port(&file) {
                return Ok(port);
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        bail!("test Chrome did not report a DevTools port within {STARTUP_TIMEOUT:?}")
    }
}

impl Drop for TestChrome {
    fn drop(&mut self) {
        stop(&self.profile, || {
            matches!(self.child.try_wait(), Ok(Some(_)) | Err(_))
        });
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.profile);
    }
}

/// Ask the Chrome on `profile` to close and wait up to [`CLOSE_WAIT`] for
/// `exited` to say it did. Blocking, for use in `Drop`; the caller kills a
/// Chrome that is still running.
pub(crate) fn stop(profile: &Path, mut exited: impl FnMut() -> bool) {
    if exited() || !request_close(profile) {
        return;
    }
    let deadline = Instant::now() + CLOSE_WAIT;
    while !exited() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Send `Browser.close` to the Chrome on `profile`, over a blocking
/// websocket to the browser endpoint its `DevToolsActivePort` names.
fn request_close(profile: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(profile.join("DevToolsActivePort")) else {
        return false;
    };
    let mut lines = text.lines();
    let (Some(port), Some(path)) = (lines.next(), lines.next()) else {
        return false;
    };
    let Ok(port) = port.trim().parse::<u16>() else {
        return false;
    };
    let timeout = Duration::from_secs(2);
    let Ok(stream) = TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), timeout) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));
    let url = format!("ws://127.0.0.1:{port}{}", path.trim());
    let Ok((mut socket, _)) = tungstenite::client::client(url.as_str(), stream) else {
        return false;
    };
    let sent = socket
        .send(Message::Text(r#"{"id":1,"method":"Browser.close"}"#.into()))
        .is_ok();
    // The reply, or the socket closing as Chrome exits.
    let _ = socket.read();
    sent
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

fn spawn(binary: &str, profile: &Path, headless: bool) -> std::io::Result<Child> {
    let scratch = profile.join("tmp");
    std::fs::create_dir_all(&scratch)?;
    let mut command = Command::new(binary);
    if headless {
        command.arg("--headless=new");
    }
    command
        .arg("--remote-debugging-port=0")
        .arg("--remote-debugging-address=127.0.0.1")
        .arg(format!("--user-data-dir={}", profile.display()))
        .args([
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-extensions",
            "--disable-sync",
            "--disable-background-networking",
            "--disable-component-update",
            // Component downloads go nowhere: one interrupted by the close
            // leaves a com.google.Chrome.chrome_chrome_url_fetcher_* dir.
            "--component-updater=url-source=http://127.0.0.1:9/",
            "--password-store=basic",
            "--use-mock-keychain",
            "about:blank",
        ]);
    if cfg!(target_os = "linux") {
        // CI containers often run as root, where the sandbox refuses to start.
        command.arg("--no-sandbox");
    }
    command
        .env("TMPDIR", &scratch)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
}

fn temp_profile() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "{PROFILE_PREFIX}{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

/// Once per test process: remove the profiles of test processes that died
/// without dropping their Chrome, stopping that Chrome when it still runs on
/// the profile. Only this harness's own profiles are touched: the prefix, a
/// dead process's id, and a Chrome whose command line names the profile.
fn reap_orphans() {
    static REAPED: Once = Once::new();
    REAPED.call_once(|| {
        // `kill` and `ps` below are Unix tools.
        if !cfg!(unix) {
            return;
        }
        let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(owner) = name
                .to_str()
                .and_then(|name| name.strip_prefix(PROFILE_PREFIX))
                .and_then(|rest| rest.split_once('-'))
                .and_then(|(pid, counter)| {
                    counter.parse::<u64>().ok()?;
                    pid.parse::<u32>().ok()
                })
            else {
                continue;
            };
            if owner == std::process::id() || alive(owner) {
                continue;
            }
            let profile = entry.path();
            if let Some(chrome) = std::fs::read_to_string(profile.join("chrome.pid"))
                .ok()
                .and_then(|pid| pid.trim().parse::<u32>().ok())
                && runs_on(chrome, &profile)
            {
                let _ = Command::new("kill")
                    .args(["-9", &chrome.to_string()])
                    .status();
            }
            let _ = std::fs::remove_dir_all(&profile);
        }
    });
}

/// Whether a process with this id exists and is ours (`kill -0`). One of
/// another user holds an id a dead test process no longer does.
fn alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Whether process `pid` is a Chrome started on `profile`.
fn runs_on(pid: u32, profile: &Path) -> bool {
    Command::new("ps")
        .args(["-o", "command=", "-p", &pid.to_string()])
        .output()
        .is_ok_and(|output| {
            String::from_utf8_lossy(&output.stdout)
                .contains(&format!("--user-data-dir={}", profile.display()))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

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
