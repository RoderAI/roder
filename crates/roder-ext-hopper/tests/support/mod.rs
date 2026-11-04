use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::{Mutex, MutexGuard};

static ENV_LOCK: Mutex<()> = Mutex::const_new(());
static NEXT_DIR: AtomicU64 = AtomicU64::new(0);
const ENV_VARS: &[&str] = &[
    "RODER_CONFIG_DIR",
    "HOPPER_API_KEY",
    "RODER_HOPPER_API_KEY",
    "HOPPER_BASE_URL",
    "RODER_HOPPER_BASE_URL",
];

pub struct TestEnv {
    _lock: MutexGuard<'static, ()>,
    original: Vec<(&'static str, Option<OsString>)>,
    pub config_dir: PathBuf,
}

impl TestEnv {
    pub fn new() -> Self {
        Self::with_lock(ENV_LOCK.blocking_lock())
    }

    pub async fn new_async() -> Self {
        Self::with_lock(ENV_LOCK.lock().await)
    }

    fn with_lock(lock: MutexGuard<'static, ()>) -> Self {
        let config_dir = std::env::temp_dir().join(format!(
            "roder-hopper-tests-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed),
        ));
        std::fs::create_dir_all(&config_dir).unwrap();
        let original = ENV_VARS
            .iter()
            .map(|&key| (key, std::env::var_os(key)))
            .collect();
        // SAFETY: every test in this binary holds ENV_LOCK before accessing
        // Hopper configuration, and restores the environment before unlocking.
        unsafe {
            for key in ENV_VARS {
                std::env::remove_var(key);
            }
            std::env::set_var("RODER_CONFIG_DIR", &config_dir);
        }
        Self {
            _lock: lock,
            original,
            config_dir,
        }
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        // SAFETY: the environment lock remains held until after restoration.
        unsafe {
            for (key, value) in &self.original {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
        let _ = std::fs::remove_dir_all(&self.config_dir);
    }
}
