//! Finding or starting the Chrome a Jev task drives.
//!
//! `JEV_CDP_URL` names a browser to attach to, local or remote. Without it,
//! Roder reuses a DevTools endpoint already listening on the documented
//! loopback port (9222, or `JEV_CDP_PORT`), then one a Chrome running on
//! Jev's own profile advertises in its `DevToolsActivePort` file, and
//! otherwise starts a visible Chrome on that profile with a port Chrome picks
//! itself, read back from the same file. The launch follows fastbrowse's
//! adapters/local_chrome.py (MIT): the child is polled, with its stderr kept
//! in `chrome-stderr.log` beside the profile.
//!
//! Launches on a profile are serialised, within the process and across
//! processes (a lock file beside the profile), and each re-checks the
//! profile's advertised port once it holds the lock, so two tasks starting at
//! once start one Chrome. The port file is never deleted: a Chrome that
//! exits at once has handed off to one already running on the profile, and
//! that Chrome's file is how it is found again, now and by later tasks.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, bail, ensure};
use reqwest::Url;
use serde_json::{Value, json};
use tokio::process::{Child, Command};

mod lock;

const DEFAULT_CDP_PORT: u16 = 9222;
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(20);
const STARTUP_POLL: Duration = Duration::from_millis(100);
/// How long a Chrome the launch handed off to may take to answer DevTools.
const HANDOFF_WAIT: Duration = Duration::from_secs(3);
/// Where a launched Chrome's stderr goes, inside its profile.
const STDERR_LOG: &str = "chrome-stderr.log";
/// How much of that log a failed launch quotes.
const LOG_TAIL_CHARS: usize = 1500;

/// The DevTools endpoint a task attaches to: an HTTP address that advertises
/// the browser websocket, or that websocket itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChromeEndpoint {
    url: String,
    launched: bool,
}

impl ChromeEndpoint {
    pub(crate) fn new(url: impl Into<String>, launched: bool) -> Self {
        Self {
            url: url.into(),
            launched,
        }
    }

    pub(crate) fn url(&self) -> &str {
        &self.url
    }

    pub(crate) fn launched(&self) -> bool {
        self.launched
    }

    /// The endpoint as the tool result may show it. A remote browser
    /// service's address often carries a token in its path or query, so
    /// anything not on loopback is cut to its scheme and host.
    pub(crate) fn reported_url(&self) -> String {
        let Ok(url) = Url::parse(&self.url) else {
            return String::new();
        };
        let host = url.host_str().unwrap_or_default();
        let loopback = host == "localhost"
            || host
                .trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback());
        if loopback && url.username().is_empty() && url.password().is_none() {
            return self.url.clone();
        }
        format!("{}://{host}", url.scheme())
    }
}

/// Resolve a CDP endpoint, starting Chrome when nothing is listening.
pub(crate) async fn ensure() -> anyhow::Result<ChromeEndpoint> {
    if let Some(configured) = env_value("JEV_CDP_URL") {
        return Ok(ChromeEndpoint::new(configured_url(&configured)?, false));
    }
    let port = cdp_port(env_value("JEV_CDP_PORT").as_deref());
    let url = http_endpoint(port);
    if probe(&url).await {
        return Ok(ChromeEndpoint::new(url, false));
    }
    let profile = profile_dir();
    if let Some(url) = advertised(&profile).await {
        return Ok(ChromeEndpoint::new(url, false));
    }
    if !autostart_enabled(env_value("JEV_CHROME_AUTOSTART").as_deref()) {
        bail!(
            "no Chrome DevTools endpoint on 127.0.0.1:{port} or advertised by {} and \
             JEV_CHROME_AUTOSTART is off; start Chrome with --remote-debugging-port={port} or set \
             JEV_CDP_URL",
            profile.display()
        );
    }
    let candidates = chrome_candidates(env_value("JEV_CHROME_BINARY").as_deref());
    // Left running: later tasks reuse this browser and its sign-ins.
    let (endpoint, _child) = on_profile(&profile, &candidates, &[]).await?;
    Ok(endpoint)
}

/// The Chrome running on `profile`, else one started there with the first
/// of `candidates` that starts. Launches are serialised (see the module
/// docs), so a task that waited for another's launch reuses its Chrome. The
/// child is returned for a Chrome this started, so a test can stop it.
pub(crate) async fn on_profile(
    profile: &Path,
    candidates: &[String],
    extra: &[&str],
) -> anyhow::Result<(ChromeEndpoint, Option<Child>)> {
    if let Some(url) = advertised(profile).await {
        return Ok((ChromeEndpoint::new(url, false), None));
    }
    let _lock = lock::LaunchLock::acquire(profile, STARTUP_TIMEOUT * 2).await?;
    if let Some(url) = advertised(profile).await {
        return Ok((ChromeEndpoint::new(url, false), None));
    }
    let Launched { url, child } = launch_with(candidates, profile, extra).await?;
    let launched = child.is_some();
    Ok((ChromeEndpoint::new(url, launched), child))
}

/// `JEV_CDP_URL`: an http(s) DevTools address or a ws(s) browser websocket,
/// with a host. The value is never echoed, since it may carry a token.
fn configured_url(raw: &str) -> anyhow::Result<String> {
    let raw = raw.trim();
    let url = Url::parse(raw).ok();
    ensure!(
        url.as_ref().is_some_and(|url| {
            matches!(url.scheme(), "http" | "https" | "ws" | "wss")
                && url.host_str().is_some_and(|host| !host.is_empty())
        }),
        "JEV_CDP_URL must be an http(s) DevTools address or a ws(s) browser websocket URL"
    );
    Ok(raw.to_string())
}

/// The endpoint a Chrome on `profile` advertises, if it answers.
pub(crate) async fn advertised(profile: &Path) -> Option<String> {
    let port = read_active_port(&profile.join("DevToolsActivePort"))?;
    let url = http_endpoint(port);
    probe(&url).await.then_some(url)
}

/// A Chrome to use on a profile: one Roder started, with its process (left
/// running when dropped; tests kill it), or, when the one Roder started
/// handed off to a Chrome already running there, that Chrome and no process.
pub(crate) struct Launched {
    pub(crate) url: String,
    pub(crate) child: Option<Child>,
}

/// Start Chrome on `profile` with `extra` arguments, trying these binaries
/// in turn, and wait for a debuggable page. A profile this creates gets its
/// password manager turned off first; an existing one is left as its owner
/// set it. Callers that may race hold the launch lock ([`on_profile`]).
pub(crate) async fn launch_with(
    candidates: &[String],
    profile: &Path,
    extra: &[&str],
) -> anyhow::Result<Launched> {
    let created = !profile.exists();
    std::fs::create_dir_all(profile)
        .with_context(|| format!("create the Chrome profile {}", profile.display()))?;
    if created {
        quiet_password_manager(profile)?;
    }
    let mut failures = Vec::new();
    for binary in candidates {
        // What the port file said before this launch: only a change is the
        // new Chrome's port. It is not deleted, since a Chrome already
        // running on the profile keeps its port there.
        let before = PortFile::read(profile);
        match spawn(binary, profile, extra) {
            Ok(child) => return wait_until_ready(child, profile, &before).await,
            Err(error) => failures.push(format!("{binary}: {error}")),
        }
    }
    bail!(
        "could not start Chrome for jev_browse (tried {}); install Chrome, set JEV_CHROME_BINARY, \
         or start Chrome yourself with --remote-debugging-port=9222",
        failures.join("; ")
    )
}

fn spawn(binary: &str, profile: &Path, extra: &[&str]) -> anyhow::Result<Child> {
    let log = std::fs::File::create(profile.join(STDERR_LOG))
        .with_context(|| format!("create {STDERR_LOG}"))?;
    // Detached on purpose: the browser outlives the task so later calls reuse
    // it, and the user keeps the window and its signed-in profile.
    Command::new(binary)
        .arg("--remote-debugging-port=0")
        .arg(format!("--user-data-dir={}", profile.display()))
        .args(["--no-first-run", "--no-default-browser-check"])
        .args(extra)
        .arg("about:blank")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log)
        .spawn()
        .with_context(|| format!("spawn {binary}"))
}

/// `DevToolsActivePort` as last seen: its contents and when it was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PortFile(Option<(String, Option<std::time::SystemTime>)>);

impl PortFile {
    pub(crate) fn read(profile: &Path) -> Self {
        let file = profile.join("DevToolsActivePort");
        Self(std::fs::read_to_string(&file).ok().map(|contents| {
            let written = std::fs::metadata(&file)
                .and_then(|meta| meta.modified())
                .ok();
            (contents, written)
        }))
    }
}

/// A just-started Chrome answers `/json/version` before it can evaluate
/// script in a tab, so this waits for a page target as well, polling the
/// child. Only a port file that changed since `before` is this Chrome's. A
/// child that exits has usually handed off to a Chrome already running on
/// the profile (its `SingletonLock` is held); that Chrome is used when its
/// port answers, and otherwise the launch fails with the end of the log.
async fn wait_until_ready(
    mut child: Child,
    profile: &Path,
    before: &PortFile,
) -> anyhow::Result<Launched> {
    let deadline = tokio::time::Instant::now() + STARTUP_TIMEOUT;
    let file = profile.join("DevToolsActivePort");
    while tokio::time::Instant::now() < deadline {
        if let Some(status) = child.try_wait()? {
            if let Some(url) = running(profile).await {
                return Ok(Launched { url, child: None });
            }
            let held = match singleton_held(profile) {
                true => "a Chrome is running on it without answering DevTools",
                false => "is a Chrome without remote debugging already running on it?",
            };
            bail!(
                "Chrome exited ({status}) before DevTools started on {}; {held}{}",
                profile.display(),
                log_tail(profile)
            );
        }
        if &PortFile::read(profile) != before
            && let Some(port) = read_active_port(&file)
        {
            let url = http_endpoint(port);
            if probe(&url).await && has_page_target(&url).await {
                return Ok(Launched {
                    url,
                    child: Some(child),
                });
            }
        }
        tokio::time::sleep(STARTUP_POLL).await;
    }
    bail!(
        "Chrome started but did not expose a debuggable page within {} seconds{}",
        STARTUP_TIMEOUT.as_secs(),
        log_tail(profile)
    )
}

/// The endpoint of a Chrome already running on `profile`. One holding the
/// profile's `SingletonLock` may be busy, so its advertised port is tried
/// for a few seconds; without the lock, nothing runs there to wait for.
async fn running(profile: &Path) -> Option<String> {
    let deadline = tokio::time::Instant::now() + HANDOFF_WAIT;
    loop {
        if let Some(url) = advertised(profile).await {
            return Some(url);
        }
        if !singleton_held(profile) || tokio::time::Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(STARTUP_POLL).await;
    }
}

/// Chrome holds `SingletonLock` (a symlink naming its host and process)
/// while it runs on a profile.
fn singleton_held(profile: &Path) -> bool {
    profile.join("SingletonLock").symlink_metadata().is_ok()
}

fn log_tail(profile: &Path) -> String {
    let log = std::fs::read_to_string(profile.join(STDERR_LOG)).unwrap_or_default();
    let log = log.trim();
    if log.is_empty() {
        return String::new();
    }
    let skip = log.chars().count().saturating_sub(LOG_TAIL_CHARS);
    format!(":\n{}", log.chars().skip(skip).collect::<String>())
}

/// The first line of `DevToolsActivePort` is the port; the file can be seen
/// half-written, so anything unparsable means "not yet".
pub(crate) fn read_active_port(file: &Path) -> Option<u16> {
    std::fs::read_to_string(file)
        .ok()?
        .lines()
        .next()?
        .trim()
        .parse()
        .ok()
        .filter(|port| *port > 0)
}

/// Turn off Chrome's password manager in a profile Roder is creating. After
/// a sign-in, its save and leaked-password bubbles are Chrome's own
/// interface, outside the page, and swallow the clicks that follow
/// (fastbrowse saw it, MIT). Only a new profile is written: an existing one
/// may be the user's, with settings of their own.
fn quiet_password_manager(profile: &Path) -> anyhow::Result<()> {
    let preferences = profile.join("Default").join("Preferences");
    std::fs::create_dir_all(profile.join("Default"))?;
    std::fs::write(&preferences, password_manager_off().to_string())
        .with_context(|| format!("write {}", preferences.display()))
}

fn password_manager_off() -> Value {
    json!({
        "credentials_enable_service": false,
        "profile": {
            "password_manager_enabled": false,
            "password_manager_leak_detection": false,
        },
    })
}

async fn has_page_target(url: &str) -> bool {
    let Ok(client) = reqwest::Client::builder().timeout(PROBE_TIMEOUT).build() else {
        return false;
    };
    let Ok(response) = client.get(format!("{url}/json/list")).send().await else {
        return false;
    };
    let Ok(targets) = response.json::<serde_json::Value>().await else {
        return false;
    };
    targets.as_array().is_some_and(|targets| {
        targets.iter().any(|target| {
            target["type"] == "page"
                && target["webSocketDebuggerUrl"]
                    .as_str()
                    .is_some_and(|url| !url.is_empty())
        })
    })
}

async fn probe(url: &str) -> bool {
    let Ok(client) = reqwest::Client::builder().timeout(PROBE_TIMEOUT).build() else {
        return false;
    };
    let Ok(response) = client.get(format!("{url}/json/version")).send().await else {
        return false;
    };
    if !response.status().is_success() {
        return false;
    }
    response
        .text()
        .await
        .is_ok_and(|body| body.contains("webSocketDebuggerUrl"))
}

fn env_value(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn profile_dir() -> PathBuf {
    roder_config::config_dir().join("jev-chrome")
}

fn http_endpoint(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

fn cdp_port(raw: Option<&str>) -> u16 {
    raw.and_then(|value| value.trim().parse::<u16>().ok())
        .filter(|port| *port > 0)
        .unwrap_or(DEFAULT_CDP_PORT)
}

fn autostart_enabled(raw: Option<&str>) -> bool {
    match raw.map(|value| value.trim().to_ascii_lowercase()) {
        Some(value) => !matches!(value.as_str(), "0" | "false" | "no" | "off"),
        None => true,
    }
}

pub(crate) fn chrome_candidates(explicit: Option<&str>) -> Vec<String> {
    if let Some(binary) = explicit.map(str::trim).filter(|value| !value.is_empty()) {
        return vec![binary.to_string()];
    }
    #[cfg(target_os = "macos")]
    let defaults = [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "google-chrome",
        "chromium",
    ];
    #[cfg(target_os = "windows")]
    let defaults = [
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
        "chrome.exe",
    ];
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let defaults = [
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
    ];
    defaults.iter().map(|path| path.to_string()).collect()
}

#[cfg(test)]
mod tests;
