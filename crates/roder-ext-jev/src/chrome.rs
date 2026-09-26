//! Chrome CDP endpoint discovery for Jev browser tasks.
//!
//! Jev talks to Chrome through browser-harness, which needs a DevTools
//! endpoint. Roder reuses one when it is already listening and otherwise
//! starts a visible Chrome on its own profile, so a browser task never fails
//! just because no debuggable browser happens to be running.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, bail};
use tokio::process::Command;

const DEFAULT_CDP_PORT: u16 = 9222;
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(20);
const STARTUP_POLL: Duration = Duration::from_millis(250);
const STARTUP_SETTLE: Duration = Duration::from_millis(750);

/// How the Jev child process should reach Chrome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ChromeEndpoint {
    /// `BU_CDP_URL` / `BU_CDP_WS` is already set; pass the environment through.
    Inherited,
    /// Roder resolved a DevTools HTTP endpoint for the child to attach to.
    Attached { url: String, launched: bool },
}

impl ChromeEndpoint {
    pub(crate) fn url(&self) -> Option<&str> {
        match self {
            Self::Inherited => None,
            Self::Attached { url, .. } => Some(url),
        }
    }

    pub(crate) fn launched(&self) -> bool {
        matches!(self, Self::Attached { launched: true, .. })
    }
}

/// Resolve a CDP endpoint, starting Chrome when nothing is listening.
pub(crate) async fn ensure() -> anyhow::Result<ChromeEndpoint> {
    if inherits_endpoint(
        env_value("BU_CDP_URL").as_deref(),
        env_value("BU_CDP_WS").as_deref(),
    ) {
        return Ok(ChromeEndpoint::Inherited);
    }
    let port = cdp_port(env_value("JEV_CDP_PORT").as_deref());
    let url = http_endpoint(port);
    if probe(&url).await {
        return Ok(ChromeEndpoint::Attached {
            url,
            launched: false,
        });
    }
    if !autostart_enabled(env_value("JEV_CHROME_AUTOSTART").as_deref()) {
        bail!(
            "no Chrome DevTools endpoint on 127.0.0.1:{port} and JEV_CHROME_AUTOSTART is off; \
             start Chrome with --remote-debugging-port={port} or set BU_CDP_URL"
        );
    }
    launch(port).await?;
    Ok(ChromeEndpoint::Attached {
        url,
        launched: true,
    })
}

async fn launch(port: u16) -> anyhow::Result<()> {
    let profile = profile_dir();
    let candidates = chrome_candidates(env_value("JEV_CHROME_BINARY").as_deref());
    let mut failures = Vec::new();
    for binary in &candidates {
        match spawn(binary, port, &profile).await {
            Ok(()) => return wait_until_ready(port).await,
            Err(error) => failures.push(format!("{binary}: {error}")),
        }
    }
    bail!(
        "could not start Chrome for jev_browse (tried {}); install Chrome, set JEV_CHROME_BINARY, \
         or start Chrome yourself with --remote-debugging-port={port}",
        failures.join("; ")
    )
}

async fn spawn(binary: &str, port: u16, profile: &Path) -> anyhow::Result<()> {
    // Detached on purpose: the browser outlives the task so later calls reuse
    // it, and the user keeps the window and its signed-in profile.
    Command::new(binary)
        .arg(format!("--remote-debugging-port={port}"))
        .arg(format!("--user-data-dir={}", profile.display()))
        .args([
            "--no-first-run",
            "--no-default-browser-check",
            "about:blank",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .with_context(|| format!("spawn {binary}"))
}

/// A just-started Chrome answers `/json/version` before it can evaluate script
/// in a tab, which makes the first browser task fail. Wait for a real page
/// target as well, then let the browser settle.
async fn wait_until_ready(port: u16) -> anyhow::Result<()> {
    let url = http_endpoint(port);
    let deadline = tokio::time::Instant::now() + STARTUP_TIMEOUT;
    while tokio::time::Instant::now() < deadline {
        if probe(&url).await && has_page_target(&url).await {
            tokio::time::sleep(STARTUP_SETTLE).await;
            return Ok(());
        }
        tokio::time::sleep(STARTUP_POLL).await;
    }
    bail!(
        "Chrome started but did not expose a debuggable page on 127.0.0.1:{port} within {} seconds",
        STARTUP_TIMEOUT.as_secs()
    )
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

fn inherits_endpoint(cdp_url: Option<&str>, cdp_ws: Option<&str>) -> bool {
    [cdp_url, cdp_ws]
        .into_iter()
        .flatten()
        .any(|value| !value.trim().is_empty())
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

fn chrome_candidates(explicit: Option<&str>) -> Vec<String> {
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
mod tests {
    use super::*;

    #[test]
    fn inherited_endpoint_wins_over_discovery() {
        assert!(inherits_endpoint(Some("http://127.0.0.1:9333"), None));
        assert!(inherits_endpoint(
            None,
            Some("ws://127.0.0.1:9333/devtools")
        ));
        assert!(!inherits_endpoint(None, None));
        assert!(!inherits_endpoint(Some("  "), Some("")));
    }

    #[test]
    fn cdp_port_defaults_and_validates() {
        assert_eq!(cdp_port(None), DEFAULT_CDP_PORT);
        assert_eq!(cdp_port(Some(" 9333 ")), 9333);
        assert_eq!(cdp_port(Some("0")), DEFAULT_CDP_PORT);
        assert_eq!(cdp_port(Some("not-a-port")), DEFAULT_CDP_PORT);
    }

    #[test]
    fn autostart_is_on_unless_explicitly_disabled() {
        assert!(autostart_enabled(None));
        assert!(autostart_enabled(Some("1")));
        assert!(!autostart_enabled(Some("0")));
        assert!(!autostart_enabled(Some(" Off ")));
        assert!(!autostart_enabled(Some("false")));
    }

    #[test]
    fn explicit_binary_replaces_platform_candidates() {
        assert_eq!(
            chrome_candidates(Some("/opt/chrome/chrome")),
            vec!["/opt/chrome/chrome".to_string()]
        );
        assert!(chrome_candidates(Some("   ")).len() > 1);
        assert!(!chrome_candidates(None).is_empty());
    }

    #[test]
    fn attached_endpoint_reports_launch_state() {
        let reused = ChromeEndpoint::Attached {
            url: "http://127.0.0.1:9222".into(),
            launched: false,
        };
        assert_eq!(reused.url(), Some("http://127.0.0.1:9222"));
        assert!(!reused.launched());
        assert!(ChromeEndpoint::Inherited.url().is_none());
    }
}
