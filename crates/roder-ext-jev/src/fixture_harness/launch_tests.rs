//! Starting Chrome the way `jev_browse` does, on a throwaway profile.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::process::Child;

use super::browser::{binaries, stop};
use super::site::FixtureSite;
use crate::cdp::Connection;
use crate::chrome;
use crate::page::Page;

fn profile(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("roder-jev-launch-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Headless, quiet about the keychain, and with component downloads going
/// nowhere (one cut off by the close leaves a temporary directory), as the
/// fixture Chromes are.
fn headless() -> Vec<&'static str> {
    let mut extra = vec![
        "--headless=new",
        "--password-store=basic",
        "--use-mock-keychain",
        "--component-updater=url-source=http://127.0.0.1:9/",
    ];
    if cfg!(target_os = "linux") {
        extra.push("--no-sandbox");
    }
    extra
}

/// A Chrome these tests started, closed as a user would quit it when the
/// test ends (a kill leaves Chrome's temporary directory behind), killed if
/// it will not close, and its profile removed; on a panic too.
struct Started {
    child: Option<Child>,
    profile: PathBuf,
}

impl Started {
    fn new(child: Option<Child>, profile: &Path) -> Self {
        Self {
            child,
            profile: profile.to_path_buf(),
        }
    }
}

impl Drop for Started {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            stop(&self.profile, || {
                matches!(child.try_wait(), Ok(Some(_)) | Err(_))
            });
            let _ = child.start_kill();
        }
        // The Chrome may take a moment to let go of its files.
        for _ in 0..20 {
            if std::fs::remove_dir_all(&self.profile).is_ok() || !self.profile.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let mut lock = self.profile.as_os_str().to_os_string();
        lock.push(".launch-lock");
        let _ = std::fs::remove_file(lock);
    }
}

#[tokio::test]
async fn a_launched_chrome_is_found_by_its_profile_and_used_at_once() {
    let Some(binaries) = binaries().expect("a usable Chrome") else {
        return;
    };
    let dir = profile("ready");
    let launched = chrome::launch_with(&binaries, &dir, &headless())
        .await
        .expect("launch Chrome");
    let url = launched.url.clone();
    let _chrome = Started::new(launched.child, &dir);
    // A profile Jev created has its password manager off, and the log is
    // kept beside it.
    let preferences: Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("Default/Preferences")).unwrap_or_default(),
    )
    .unwrap_or(Value::Null);
    assert_eq!(preferences["credentials_enable_service"], json!(false));
    assert!(dir.join("chrome-stderr.log").is_file());
    // A later task finds this Chrome through DevToolsActivePort.
    assert_eq!(
        chrome::advertised(&dir).await.as_deref(),
        Some(url.as_str())
    );
    // No startup settle: the first task opens and reads a page at once.
    let site = FixtureSite::start().await.expect("fixture site");
    let connection = Connection::connect(&url).await.expect("connect");
    let mut page = Page::open(connection, &site.url("basic.html"), false)
        .await
        .expect("open a page straight after launch");
    let observation = page.observe().await.expect("observe");
    page.close().await.ok();
    assert_eq!(observation["title"], json!("Basic controls"));
}

/// A launch on a profile whose Chrome is running (the probe missed it once,
/// busy) hands off to that Chrome and exits. That Chrome is used, and its
/// port file is kept, so later tasks still find it; the launch used to
/// delete the file first and fail, and every later task failed with it.
#[tokio::test]
async fn a_launch_on_a_running_profile_reuses_that_chrome_and_keeps_its_port_file() {
    let Some(binaries) = binaries().expect("a usable Chrome") else {
        return;
    };
    let dir = profile("handoff");
    let first = chrome::launch_with(&binaries, &dir, &headless())
        .await
        .expect("launch Chrome");
    let url = first.url.clone();
    let _chrome = Started::new(first.child, &dir);
    let again = chrome::launch_with(&binaries, &dir, &headless())
        .await
        .expect("a second launch reuses the running Chrome");
    assert_eq!(again.url, url);
    assert!(again.child.is_none(), "the second launch kept a process");
    assert_eq!(
        chrome::advertised(&dir).await.as_deref(),
        Some(url.as_str())
    );
}

/// Two tasks that find no Chrome at once start one, and both use it.
#[tokio::test]
async fn tasks_starting_at_once_start_one_chrome() {
    let Some(binaries) = binaries().expect("a usable Chrome") else {
        return;
    };
    let dir = profile("race");
    let extra = headless();
    let (a, b) = tokio::join!(
        chrome::on_profile(&dir, &binaries, &extra),
        chrome::on_profile(&dir, &binaries, &extra),
    );
    let ((a, a_child), (b, b_child)) = (a.expect("first task"), b.expect("second task"));
    let started = [a.launched(), b.launched()];
    let _chrome = Started::new(a_child.or(b_child), &dir);
    assert_eq!(a.url(), b.url());
    assert_eq!(
        started.iter().filter(|launched| **launched).count(),
        1,
        "{started:?}"
    );
}

#[tokio::test]
async fn a_chrome_that_exits_fails_at_once_and_an_existing_profile_is_left_alone() {
    let exits = ["/usr/bin/false", "/bin/false"]
        .into_iter()
        .find(|binary| std::path::Path::new(binary).is_file());
    let Some(exits) = exits else {
        eprintln!("skipping: no `false` binary");
        return;
    };
    let dir = profile("exits");
    std::fs::create_dir_all(dir.join("Default")).unwrap();
    let theirs = r#"{"profile":{"password_manager_enabled":true}}"#;
    std::fs::write(dir.join("Default/Preferences"), theirs).unwrap();
    let started = std::time::Instant::now();
    let error = match chrome::launch_with(&[exits.to_string()], &dir, &[]).await {
        Ok(_) => panic!("a binary that exits was reported as a running Chrome"),
        Err(error) => error.to_string(),
    };
    assert!(error.contains("exited"), "{error}");
    // No Chrome holds the profile, so there is no hand-off to wait for.
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("Default/Preferences")).unwrap(),
        theirs
    );
    std::fs::remove_dir_all(&dir).ok();
}
