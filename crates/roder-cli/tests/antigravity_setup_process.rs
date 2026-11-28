//! Noninteractive entry points must not install software or start Google login.

#![cfg(feature = "antigravity")]

use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct Fixture {
    root: PathBuf,
    child: Option<Child>,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "roder-antigravity-setup-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("empty-path")).unwrap();
        Self { root, child: None }
    }

    fn run(&mut self, args: &[&str]) -> Output {
        self.child = Some(
            Command::new(env!("CARGO_BIN_EXE_roder"))
                .args(args)
                .current_dir(&self.root)
                .env_clear()
                .env("HOME", &self.root)
                .env("USERPROFILE", &self.root)
                .env("RODER_CONFIG_DIR", self.root.join("config"))
                .env("RODER_DATA_DIR", self.root.join("data"))
                .env("PATH", self.root.join("empty-path"))
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("start isolated roder"),
        );
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if self.child.as_mut().unwrap().try_wait().unwrap().is_some() {
                return self.child.take().unwrap().wait_with_output().unwrap();
            }
            assert!(
                Instant::now() < deadline,
                "missing-runtime setup must not hang or wait for input"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn missing_antigravity_in_a_pipe_explains_guided_setup_without_installing() {
    for args in [
        vec!["--backend", "antigravity"],
        vec!["--backend=antigravity", "--yolo"],
        vec!["setup", "antigravity"],
    ] {
        let mut fixture = Fixture::new();
        let output = fixture.run(&args);
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{output:?}");
        assert!(
            error.contains("setup requires an interactive terminal"),
            "{error}"
        );
        assert!(error.contains("guided installation"), "{error}");
        assert!(
            error.contains("No download or sign-in was started"),
            "{error}"
        );
        assert!(
            !fixture.root.join("config").exists(),
            "checking setup must not create a managed installation"
        );
        assert!(!fixture.root.join("data").exists());
    }
}

#[test]
fn missing_runtime_login_points_to_the_setup_wizard() {
    let mut fixture = Fixture::new();
    let output = fixture.run(&["auth", "login", "antigravity"]);
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "{output:?}");
    assert!(error.contains("guided installation and sign-in"), "{error}");
    assert!(!fixture.root.join("config").exists());
}

#[test]
fn setup_rejects_unrecognized_flags_instead_of_bypassing_consent() {
    let mut fixture = Fixture::new();
    let output = fixture.run(&["setup", "antigravity", "--yes"]);
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "{output:?}");
    assert!(error.contains("usage: roder setup antigravity"), "{error}");
    assert!(!fixture.root.join("config").exists());
}

#[test]
#[cfg(all(
    any(target_os = "linux", target_os = "macos", target_os = "windows"),
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
fn damaged_managed_record_still_explains_how_to_repair_in_the_wizard() {
    let mut fixture = Fixture::new();
    let os = if cfg!(target_os = "macos") {
        "darwin"
    } else {
        std::env::consts::OS
    };
    let arch = if cfg!(target_arch = "x86_64") {
        "amd64"
    } else {
        "arm64"
    };
    let suffix = if cfg!(all(target_os = "linux", target_env = "musl")) {
        "_musl"
    } else {
        ""
    };
    let managed = fixture
        .root
        .join("config/tools/antigravity")
        .join(format!("{os}_{arch}{suffix}"));
    std::fs::create_dir_all(&managed).unwrap();
    let record = managed.join("active.json");
    let damaged = r#"{"release":"../../outside"}"#;
    std::fs::write(&record, damaged).unwrap();
    let output = fixture.run(&["--backend", "antigravity"]);
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "{output:?}");
    assert!(
        error.contains("setup requires an interactive terminal"),
        "{error}"
    );
    assert!(error.contains("damaged"), "{error}");
    assert_eq!(std::fs::read_to_string(record).unwrap(), damaged);
    assert!(!managed.join("versions").exists());
}
