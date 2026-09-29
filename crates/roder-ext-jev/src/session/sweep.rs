//! Closing the tabs of sessions nobody will continue.
//!
//! There is no thread-closed hook, so a session ends when it has been idle
//! for `JEV_SESSION_IDLE_SECS` (20 minutes by default), when a ninth session
//! in the process pushes out the least recently used idle one, or when a
//! call asks for `tab: "close"`. A process that exits or crashes leaves its
//! tabs open in Jev's Chrome, which keeps running; so each process keeps a
//! ledger of its sessions' tabs under `jev-sessions/` in Roder's config
//! directory, rewritten after every call and sweep, and any process's
//! sweeper closes the tabs of a ledger nobody has refreshed for twice the
//! idle limit and deletes it. Only a loopback DevTools endpoint is written
//! down, never an address that may carry a token, so a remote browser's tabs
//! are left to it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::cdp::Connection;
use crate::chrome::ChromeEndpoint;
use crate::runner::{close_all, close_quietly};

/// This process's ledger file, and the directory other processes' live in.
pub(crate) struct DiskLedger {
    dir: PathBuf,
    own: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
struct LedgerFile {
    /// Seconds since the Unix epoch.
    updated_at: u64,
    browsers: Vec<LedgerBrowser>,
}

#[derive(Debug, Serialize, Deserialize)]
struct LedgerBrowser {
    endpoint: String,
    targets: Vec<String>,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// The endpoint as a ledger may keep it: only a loopback address without
/// credentials, which [`ChromeEndpoint::reported_url`] leaves whole.
pub(crate) fn recordable(endpoint: &ChromeEndpoint) -> Option<String> {
    let reported = endpoint.reported_url();
    (reported == endpoint.url()).then_some(reported)
}

impl DiskLedger {
    /// A ledger for this process in `dir`.
    pub(crate) fn new(dir: PathBuf) -> Self {
        let name = format!("{}-{:x}.json", std::process::id(), unique());
        Self {
            own: dir.join(name),
            dir,
        }
    }

    #[cfg(test)]
    pub(crate) fn own_path(&self) -> &Path {
        &self.own
    }

    /// Record the tabs this process's sessions hold, by browser; with none,
    /// the file is removed.
    pub(crate) fn write(&self, browsers: &BTreeMap<String, Vec<String>>) {
        let browsers = browsers
            .iter()
            .filter(|(_, targets)| !targets.is_empty())
            .map(|(endpoint, targets)| LedgerBrowser {
                endpoint: endpoint.clone(),
                targets: targets.clone(),
            })
            .collect::<Vec<_>>();
        if browsers.is_empty() {
            let _ = std::fs::remove_file(&self.own);
            return;
        }
        let file = LedgerFile {
            updated_at: now_secs(),
            browsers,
        };
        if std::fs::create_dir_all(&self.dir).is_ok()
            && let Ok(json) = serde_json::to_string(&file)
        {
            let _ = std::fs::write(&self.own, json);
        }
    }

    /// Close the tabs of every other ledger older than `max_age` and delete
    /// it. Returns how many ledgers were swept.
    pub(crate) async fn sweep_dead(&self, max_age: Duration) -> usize {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return 0;
        };
        let cutoff = now_secs().saturating_sub(max_age.as_secs());
        let mut swept = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            if path == self.own || path.extension().is_none_or(|ext| ext != "json") {
                continue;
            }
            let Some(file) = read(&path) else {
                // Unreadable: removed once it is as old as a dead ledger.
                if modified_secs(&path).is_some_and(|at| at < cutoff) {
                    let _ = std::fs::remove_file(&path);
                }
                continue;
            };
            if file.updated_at >= cutoff {
                continue;
            }
            for browser in &file.browsers {
                close_quietly(async {
                    let mut connection = Connection::connect(&browser.endpoint).await?;
                    close_all(&mut connection, &browser.targets).await;
                    Ok(())
                })
                .await;
            }
            let _ = std::fs::remove_file(&path);
            swept += 1;
        }
        swept
    }
}

fn read(path: &Path) -> Option<LedgerFile> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn modified_secs(path: &Path) -> Option<u64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(modified.duration_since(UNIX_EPOCH).ok()?.as_secs())
}

/// A value unlikely to repeat between two processes with the same pid.
fn unique() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u64(now_secs());
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "roder-jev-ledger-{name}-{}-{:x}",
            std::process::id(),
            unique()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn only_a_loopback_endpoint_is_written_down() {
        let local = ChromeEndpoint::new("http://127.0.0.1:9222", true);
        assert_eq!(recordable(&local).as_deref(), Some("http://127.0.0.1:9222"));
        let remote = ChromeEndpoint::new("wss://browser.example.com/devtools?token=t", false);
        assert_eq!(recordable(&remote), None);
    }

    #[tokio::test]
    async fn a_fresh_ledger_is_kept_and_an_empty_one_removed() {
        let dir = temp_dir("fresh");
        let ours = DiskLedger::new(dir.clone());
        let theirs = DiskLedger::new(dir.clone());
        let mut browsers = BTreeMap::new();
        browsers.insert("http://127.0.0.1:1".to_string(), vec!["T1".to_string()]);
        theirs.write(&browsers);
        assert!(theirs.own_path().is_file());
        // Written just now: not dead yet.
        assert_eq!(ours.sweep_dead(Duration::from_secs(60)).await, 0);
        assert!(theirs.own_path().is_file());
        theirs.write(&BTreeMap::new());
        assert!(!theirs.own_path().exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
