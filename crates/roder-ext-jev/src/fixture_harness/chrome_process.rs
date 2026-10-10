//! One throwaway Chrome process: started on a fresh temporary profile under
//! a watcher, found by its `DevToolsActivePort` file, asked to close.
//!
//! Chrome gets a free DevTools port (`--remote-debugging-port=0`). It never
//! probes 9222 and never touches the persistent `jev-chrome` profile.
//!
//! The watcher is what keeps a Chrome from outliving the test process: a
//! static is never dropped, so no `Drop` runs at exit, and a test process
//! that is killed (Ctrl-C of a hung run, a CI timeout, the OOM killer) runs
//! no code at all. On Unix the Chrome is started by a small shell that
//! checks once a second that the test process and the Chrome are both still
//! there, and when either is gone stops the Chrome (`TERM`, then `KILL`) and
//! removes the profile and the singleton directory a signalled Chrome leaves
//! behind ([`remove_singleton_dir`]). Such a leak used to leave Chromes
//! running for hours. [`reap_orphans`] still removes
//! what an older run left. Off Unix there is no shell to use, and the Chrome
//! is started directly and stopped only by [`ChromeProcess`]'s `Drop`.

use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Once;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::{self, Message};

/// Generous: on a loaded machine a Chrome has taken over 20 s to report its
/// port.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(60);
/// How long a Chrome asked to close may take before it is killed.
const CLOSE_WAIT: Duration = Duration::from_secs(5);
/// Every profile a test Chrome runs on starts with this, then the test
/// process's id and a counter. The reaper and the memory sampler key on it.
pub(super) const PROFILE_PREFIX: &str = "roder-jev-test-";

/// Runs the Chrome in `$JEV_TEST_CHROME` with the arguments given to the
/// shell, and ends it when the shell's parent or the Chrome is gone: `TERM`
/// first, `KILL` after four seconds, then the profile in
/// `$JEV_TEST_PROFILE`, and the directory Chrome keeps its singleton socket in
/// (see [`remove_singleton_dir`]). The Chrome's pid goes to `chrome.pid` in
/// the profile for [`reap_orphans`]. The binary is passed in the environment
/// so that the shell's own command line, which names the profile, does not
/// also read as a second browser.
#[cfg(unix)]
const WATCHER: &str = r#"
"$JEV_TEST_CHROME" "$@" &
chrome=$!
echo "$chrome" > "$JEV_TEST_PROFILE/chrome.pid"
while kill -0 "$PPID" 2>/dev/null && kill -0 "$chrome" 2>/dev/null; do sleep 1; done
kill "$chrome" 2>/dev/null
for waited in 1 2 3 4; do kill -0 "$chrome" 2>/dev/null || break; sleep 1; done
kill -9 "$chrome" 2>/dev/null
socket=$(readlink "$JEV_TEST_PROFILE/SingletonSocket" 2>/dev/null)
if [ -n "$socket" ]; then
  rm -f "${socket%/*}/SingletonSocket" "${socket%/*}/SingletonCookie"
  rmdir "${socket%/*}" 2>/dev/null
fi
rm -rf "$JEV_TEST_PROFILE"
"#;

/// A running throwaway Chrome. Dropping it closes the Chrome and removes the
/// profile; the shared one lives in a static and is never dropped, so the
/// watcher ends it.
pub(super) struct ChromeProcess {
    /// The watcher shell, or the Chrome itself where there is no shell.
    child: Child,
    profile: PathBuf,
    port: u16,
    /// The browser websocket's path, as `DevToolsActivePort` names it.
    ws_path: String,
}

impl ChromeProcess {
    /// Start a Chrome from the first of `binaries` that comes up, blocking
    /// until it reports its DevTools port. For use off the async executors:
    /// the process belongs to the test process, not to any test's runtime.
    pub(super) fn start(binaries: &[String], headless: bool) -> anyhow::Result<Self> {
        reap_orphans();
        let mut failures = Vec::new();
        for binary in binaries {
            match Self::start_binary(binary, &temp_profile(), headless) {
                Ok(process) => return Ok(process),
                Err(error) => failures.push(format!("{binary}: {error:#}")),
            }
        }
        bail!(
            "no Chrome binary could be started ({})",
            failures.join("; ")
        )
    }

    fn start_binary(binary: &str, profile: &Path, headless: bool) -> anyhow::Result<Self> {
        let child = match spawn(binary, profile, headless) {
            Ok(child) => child,
            Err(error) => {
                let _ = std::fs::remove_dir_all(profile);
                return Err(error).context("spawn");
            }
        };
        // Owned from here on: an early return still stops Chrome.
        let mut process = Self {
            child,
            profile: profile.to_path_buf(),
            port: 0,
            ws_path: String::new(),
        };
        process.wait_for_port()?;
        Ok(process)
    }

    /// The Chrome's own process id: the watcher records it, off Unix the
    /// child is the Chrome.
    fn chrome_pid(&self) -> Option<u32> {
        if !cfg!(unix) {
            return Some(self.child.id());
        }
        std::fs::read_to_string(self.profile.join("chrome.pid"))
            .ok()
            .and_then(|pid| pid.trim().parse().ok())
    }

    fn wait_for_port(&mut self) -> anyhow::Result<()> {
        let file = self.profile.join("DevToolsActivePort");
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        while Instant::now() < deadline {
            if let Some(status) = self.child.try_wait()? {
                bail!("test Chrome exited during startup: {status}");
            }
            if let Some((port, path)) = read_port_file(&file) {
                self.port = port;
                self.ws_path = path;
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        bail!("test Chrome did not report a DevTools port within {STARTUP_TIMEOUT:?}")
    }

    /// The DevTools HTTP address.
    pub(super) fn http(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// The browser websocket Chrome advertises.
    pub(super) fn websocket(&self) -> String {
        format!("ws://127.0.0.1:{}{}", self.port, self.ws_path)
    }

    /// Whether something still listens on the DevTools port: false for a
    /// Chrome that crashed or was stopped.
    pub(super) fn answers(&self) -> bool {
        TcpStream::connect_timeout(&([127, 0, 0, 1], self.port).into(), Duration::from_secs(1))
            .is_ok()
    }

    /// A blocking websocket to the browser endpoint, for `Drop`s.
    pub(super) fn blocking_socket(&self) -> Option<BlockingSocket> {
        blocking_socket(&self.profile)
    }
}

impl Drop for ChromeProcess {
    fn drop(&mut self) {
        stop(&self.profile, || {
            matches!(self.child.try_wait(), Ok(Some(_)) | Err(_))
        });
        // Still running: the Chrome would not close, or never came up. It
        // is killed first, since ending the watcher shell leaves it behind.
        if cfg!(unix)
            && matches!(self.child.try_wait(), Ok(None))
            && let Some(pid) = self.chrome_pid()
        {
            let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        remove_singleton_dir(&self.profile);
        let _ = std::fs::remove_dir_all(&self.profile);
    }
}

/// Chrome keeps its profile's singleton socket in a temporary directory of
/// its own (`com.google.Chrome.*` on macOS, where it ignores `TMPDIR`),
/// named by the profile's `SingletonSocket` link, and removes it only on a
/// clean exit. A Chrome that was stopped with a signal leaves it behind, one
/// per launch. Remove it, and only if nothing but the singleton's own two
/// files are in it. For a Chrome that has exited.
fn remove_singleton_dir(profile: &Path) {
    let Ok(socket) = std::fs::read_link(profile.join("SingletonSocket")) else {
        return;
    };
    let Some(dir) = socket.parent() else {
        return;
    };
    let _ = std::fs::remove_file(dir.join("SingletonSocket"));
    let _ = std::fs::remove_file(dir.join("SingletonCookie"));
    let _ = std::fs::remove_dir(dir);
}

/// The port and browser websocket path `DevToolsActivePort` holds. The file
/// can be seen half-written, so anything unparsable means "not yet".
fn read_port_file(file: &Path) -> Option<(u16, String)> {
    let text = std::fs::read_to_string(file).ok()?;
    let mut lines = text.lines();
    let port = lines
        .next()?
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|p| *p > 0)?;
    let path = lines.next()?.trim();
    path.starts_with('/').then(|| (port, path.to_string()))
}

/// Ask the Chrome on `profile` to close and wait up to [`CLOSE_WAIT`] for
/// `exited` to say it did. Blocking, for use in `Drop`; the caller kills a
/// Chrome that is still running.
pub(super) fn stop(profile: &Path, mut exited: impl FnMut() -> bool) {
    if exited() || !request_close(profile) {
        return;
    }
    let deadline = Instant::now() + CLOSE_WAIT;
    while !exited() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A blocking websocket to a browser endpoint.
pub(super) type BlockingSocket = tungstenite::WebSocket<TcpStream>;

/// A blocking websocket to the browser endpoint the Chrome on `profile`
/// names in its `DevToolsActivePort`.
fn blocking_socket(profile: &Path) -> Option<BlockingSocket> {
    let (port, path) = read_port_file(&profile.join("DevToolsActivePort"))?;
    let timeout = Duration::from_secs(2);
    let stream = TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), timeout).ok()?;
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));
    let url = format!("ws://127.0.0.1:{port}{path}");
    tungstenite::client::client(url.as_str(), stream)
        .ok()
        .map(|(socket, _)| socket)
}

/// Send `Browser.close` to the Chrome on `profile`.
fn request_close(profile: &Path) -> bool {
    let Some(mut socket) = blocking_socket(profile) else {
        return false;
    };
    let sent = blocking_send(&mut socket, 1, "Browser.close", &json!({}));
    // The reply, or the socket closing as Chrome exits.
    let _ = socket.read();
    sent
}

fn blocking_send(socket: &mut BlockingSocket, id: u64, method: &str, params: &Value) -> bool {
    let request = json!({"id": id, "method": method, "params": params});
    socket
        .send(Message::Text(request.to_string().into()))
        .is_ok()
}

/// One DevTools command over a blocking socket, and its result; `None` when
/// the command could not be sent, failed, or got no reply in time.
pub(super) fn blocking_call(
    socket: &mut BlockingSocket,
    id: u64,
    method: &str,
    params: &Value,
) -> Option<Value> {
    if !blocking_send(socket, id, method, params) {
        return None;
    }
    loop {
        let Message::Text(text) = socket.read().ok()? else {
            continue;
        };
        let reply: Value = serde_json::from_str(&text).ok()?;
        if reply["id"].as_u64() == Some(id) {
            return reply.get("result").cloned();
        }
    }
}

fn spawn(binary: &str, profile: &Path, headless: bool) -> std::io::Result<Child> {
    let scratch = profile.join("tmp");
    std::fs::create_dir_all(&scratch)?;
    let mut arguments = Vec::new();
    if headless {
        arguments.push("--headless=new".to_string());
    }
    arguments.extend([
        "--remote-debugging-port=0".to_string(),
        "--remote-debugging-address=127.0.0.1".to_string(),
        format!("--user-data-dir={}", profile.display()),
    ]);
    arguments.extend(
        [
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
        ]
        .map(str::to_string),
    );
    if cfg!(target_os = "linux") {
        // CI containers often run as root, where the sandbox refuses to start.
        arguments.push("--no-sandbox".to_string());
    }
    #[cfg(unix)]
    let mut command = {
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", WATCHER, "roder-jev-test-chrome"])
            .args(&arguments)
            .env("JEV_TEST_CHROME", binary)
            .env("JEV_TEST_PROFILE", profile);
        command
    };
    #[cfg(not(unix))]
    let mut command = {
        let mut command = Command::new(binary);
        command.args(&arguments);
        command
    };
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
/// without their Chrome being stopped (an older run, before the watcher, or
/// one killed together with its watcher), stopping that Chrome when it still
/// runs on the profile. Only this harness's own profiles are touched: the
/// prefix, a dead process's id, and a Chrome whose command line names the
/// profile.
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
            remove_singleton_dir(&profile);
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

#[cfg(all(test, unix))]
mod tests {
    use std::sync::Arc;

    use super::super::tab_relay::TabRelay;
    use super::*;
    use crate::cdp::Connection;
    use crate::page::Page;

    /// The watcher is what ends a Chrome when the test process is killed,
    /// which runs no code of its own. A stand-in for the test process starts
    /// the watcher over a `sleep`, is killed with SIGKILL, and the `sleep`
    /// and the profile must follow.
    #[test]
    fn a_chrome_and_its_profile_go_when_the_test_process_is_killed() {
        let profile = std::env::temp_dir().join(format!("roder-jev-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&profile);
        std::fs::create_dir_all(&profile).unwrap();
        // The directory a Chrome killed by a signal leaves its singleton in.
        let singleton = fake_singleton(&profile);
        let mut stand_in = Command::new("/bin/sh")
            .args([
                "-c",
                r#"/bin/sh -c "$1" watcher 600 & wait"#,
                "stand-in",
                WATCHER,
            ])
            .env("JEV_TEST_CHROME", "sleep")
            .env("JEV_TEST_PROFILE", &profile)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let waited_for = |what: &str, condition: &dyn Fn() -> bool| {
            let deadline = Instant::now() + Duration::from_secs(20);
            while !condition() {
                assert!(Instant::now() < deadline, "timed out waiting for {what}");
                std::thread::sleep(Duration::from_millis(50));
            }
        };
        let recorded = || {
            std::fs::read_to_string(profile.join("chrome.pid"))
                .ok()
                .and_then(|pid| pid.trim().parse::<u32>().ok())
        };
        waited_for("the watcher to record its child", &|| recorded().is_some());
        let chrome = recorded().unwrap();
        assert!(alive(chrome));

        stand_in.kill().unwrap();
        stand_in.wait().unwrap();
        waited_for("the child to be stopped", &|| !alive(chrome));
        waited_for("the profile to be removed", &|| !profile.exists());
        assert!(!singleton.exists(), "{} is left", singleton.display());
    }

    /// `profile/SingletonSocket` linking to a directory holding the
    /// singleton's two files, as Chrome leaves them. The directory.
    fn fake_singleton(profile: &Path) -> PathBuf {
        let dir = profile.with_file_name(format!(
            "{}-singleton",
            profile.file_name().unwrap().to_string_lossy()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SingletonSocket"), "").unwrap();
        std::os::unix::fs::symlink("12345", dir.join("SingletonCookie")).unwrap();
        std::os::unix::fs::symlink(dir.join("SingletonSocket"), profile.join("SingletonSocket"))
            .unwrap();
        dir
    }

    #[test]
    fn the_singleton_directory_is_removed_only_when_nothing_else_is_in_it() {
        let profile =
            std::env::temp_dir().join(format!("roder-jev-watch-{}-b", std::process::id()));
        let _ = std::fs::remove_dir_all(&profile);
        std::fs::create_dir_all(&profile).unwrap();
        let singleton = fake_singleton(&profile);
        std::fs::write(singleton.join("theirs.txt"), "not Chrome's").unwrap();
        remove_singleton_dir(&profile);
        assert!(singleton.join("theirs.txt").exists());
        assert!(!singleton.join("SingletonSocket").exists());

        std::fs::remove_file(singleton.join("theirs.txt")).unwrap();
        remove_singleton_dir(&profile);
        assert!(!singleton.exists());
        // A profile with no link has nothing to remove.
        remove_singleton_dir(&profile);
        std::fs::remove_dir_all(&profile).unwrap();
    }

    /// A Chrome of its own, as the windowed tests start: reached through a
    /// relay that leaves everything as it is, so every page is the test's,
    /// and closed with its profile removed when dropped, without waiting for
    /// the watcher.
    #[tokio::test]
    async fn a_chrome_of_its_own_is_relayed_whole_and_closed_when_dropped() {
        let Some(binaries) = super::super::browser::binaries().expect("a usable Chrome") else {
            return;
        };
        let chrome = Arc::new(ChromeProcess::start(&binaries, true).expect("start a Chrome"));
        let profile = chrome.profile.clone();
        let pid = chrome.chrome_pid().expect("the watcher records the pid");
        assert!(chrome.answers() && alive(pid));

        let relay = TabRelay::start(chrome.clone(), false).await.unwrap();
        // The page the command line opens, if it is listed yet, is among the
        // test's.
        let before = relay.owned_pages().await.unwrap().len();
        let connection = Connection::connect(relay.url()).await.unwrap();
        let mut page = Page::create(connection, false).await.unwrap();
        assert_eq!(relay.owned_pages().await.unwrap().len(), before + 1);
        page.close().await.unwrap();

        drop(relay);
        drop(chrome);
        assert!(!profile.exists(), "{} is left", profile.display());
        let deadline = Instant::now() + Duration::from_secs(10);
        while alive(pid) {
            assert!(Instant::now() < deadline, "Chrome {pid} is still running");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    #[test]
    fn a_binary_that_exits_at_once_is_an_error_naming_it() {
        let error = match ChromeProcess::start(&["/bin/false".to_string()], true) {
            Ok(_) => panic!("a binary that exits was reported as a running Chrome"),
            Err(error) => format!("{error:#}"),
        };
        assert!(error.contains("/bin/false"), "{error}");
        assert!(error.contains("exited during startup"), "{error}");
    }
}
