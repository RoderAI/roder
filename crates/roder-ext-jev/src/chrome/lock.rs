//! Serialising Chrome launches on one profile.
//!
//! A task that finds no Chrome on Jev's profile starts one. Two tasks doing
//! that at once (parallel tool calls, or two Roder processes) would each
//! start a Chrome, and the second hands off to the first and exits. So a
//! launch holds a process-wide mutex and an exclusive lock on a file beside
//! the profile, and re-checks the profile once it has both. The operating
//! system drops the file lock with its holder, so a crashed launch leaves
//! nothing to clean up.

use std::collections::HashMap;
use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use anyhow::{Context, bail};
use tokio::sync::{Mutex, OwnedMutexGuard};

/// One mutex per lock file, so launches on different profiles do not wait
/// for each other.
static LAUNCHING: LazyLock<std::sync::Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> =
    LazyLock::new(Default::default);
const POLL: Duration = Duration::from_millis(100);

/// Held while one launch on a profile runs.
pub(crate) struct LaunchLock {
    _file: File,
    _held: OwnedMutexGuard<()>,
}

impl LaunchLock {
    /// Wait up to `wait` for any other launch on `profile` to finish.
    pub(crate) async fn acquire(profile: &Path, wait: Duration) -> anyhow::Result<Self> {
        let path = lock_path(profile);
        let mutex = LAUNCHING
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry(path.clone())
            .or_default()
            .clone();
        let held = tokio::time::timeout(wait, mutex.lock_owned())
            .await
            .context("another task is still starting Chrome")?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .with_context(|| format!("open {}", path.display()))?;
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            match file.try_lock() {
                Ok(()) => break,
                Err(TryLockError::WouldBlock) if tokio::time::Instant::now() < deadline => {
                    tokio::time::sleep(POLL).await;
                }
                Err(TryLockError::WouldBlock) => bail!(
                    "another process is still starting Chrome on {} (it holds {})",
                    profile.display(),
                    path.display()
                ),
                Err(TryLockError::Error(error)) => {
                    return Err(error).with_context(|| format!("lock {}", path.display()));
                }
            }
        }
        Ok(Self {
            _file: file,
            _held: held,
        })
    }
}

/// `<profile>.launch-lock`, beside the profile rather than in it, so taking
/// the lock does not create the profile a launch then checks for.
fn lock_path(profile: &Path) -> PathBuf {
    let mut name = profile.file_name().unwrap_or_default().to_os_string();
    name.push(".launch-lock");
    profile.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lock_file_sits_beside_the_profile() {
        assert_eq!(
            lock_path(Path::new("/config/roder/jev-chrome")),
            PathBuf::from("/config/roder/jev-chrome.launch-lock")
        );
    }

    #[tokio::test]
    async fn a_second_launch_waits_for_the_first() {
        let profile = std::env::temp_dir().join(format!("roder-jev-lock-{}", std::process::id()));
        let first = LaunchLock::acquire(&profile, Duration::from_secs(1))
            .await
            .unwrap();
        let waited = LaunchLock::acquire(&profile, Duration::from_millis(300)).await;
        assert!(waited.is_err(), "a second launch did not wait");
        drop(first);
        let second = LaunchLock::acquire(&profile, Duration::from_secs(1)).await;
        assert!(second.is_ok());
        drop(second);
        std::fs::remove_file(lock_path(&profile)).ok();
    }
}
