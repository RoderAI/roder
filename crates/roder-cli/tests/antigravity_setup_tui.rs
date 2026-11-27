//! Offline, real-terminal regression tests for setup and inherited login handoff.
//!
//! Requires tmux and python3 on the host; no Google account, network, or runtime.
//! Run with:
//! `RODER_TUI_E2E=1 cargo test -p roder --test antigravity_setup_tui -- --ignored`
//! Each case owns a private tmux server and has a 60-second overall deadline.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const FIXTURE_CODE: &str = "RODER_OFFLINE_FIXTURE_CODE";

struct Fixture {
    root: PathBuf,
    tmux: PathBuf,
    socket: String,
    deadline: Instant,
    child: Option<Child>,
}

impl Fixture {
    fn new(fake_runtime: bool) -> Self {
        assert_eq!(
            std::env::var("RODER_TUI_E2E").as_deref(),
            Ok("1"),
            "set RODER_TUI_E2E=1 to opt in to isolated terminal tests"
        );
        let tmux = executable("tmux");
        let python = executable("python3");
        let unique = format!(
            "{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(format!("roder-setup-tui-{unique}"));
        std::fs::create_dir_all(root.join("bin")).unwrap();
        let fixture = Self {
            root,
            tmux,
            socket: format!("roder-setup-{unique}"),
            deadline: Instant::now() + Duration::from_secs(60),
            child: None,
        };
        if fake_runtime {
            let agy = fixture.root.join("bin/agy");
            std::fs::write(&agy, format!("#!{}\n{FAKE_AGY}", python.display())).unwrap();
            std::fs::set_permissions(&agy, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        // Keep the pane alive after Roder exits so its final output is observable.
        // The literal code below is fixture data, not a Google credential.
        let script = format!(
            "{} setup antigravity\n\
             status=$?\n\
             printf '%s' \"$status\" > {}\n\
             printf '\\nRODER_FIXTURE_EXIT:%s\\n' \"$status\"\n\
             exec {} -c 'import time; time.sleep(60)'\n",
            quote(Path::new(env!("CARGO_BIN_EXE_roder"))),
            quote(&fixture.root.join("exit-status")),
            quote(&python),
        );
        std::fs::write(fixture.root.join("run.sh"), script).unwrap();
        fixture
    }

    fn start(&mut self) {
        let command = format!("/bin/sh {}", quote(&self.root.join("run.sh")));
        let root = self.root.to_string_lossy().into_owned();
        self.tmux(&[
            "new-session",
            "-d",
            "-s",
            "setup",
            "-x",
            "100",
            "-y",
            "28",
            "-c",
            &root,
            &command,
        ]);
    }

    fn tmux(&mut self, args: &[&str]) -> Output {
        assert!(Instant::now() < self.deadline, "terminal test timed out");
        let mut command = self.command();
        command.args(args);
        self.child = Some(command.spawn().expect("start isolated tmux command"));
        loop {
            if self.child.as_mut().unwrap().try_wait().unwrap().is_some() {
                let output = self.child.take().unwrap().wait_with_output().unwrap();
                assert!(
                    output.status.success(),
                    "tmux {args:?} failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                return output;
            }
            assert!(Instant::now() < self.deadline, "tmux command timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.tmux);
        command
            .args(["-L", &self.socket, "-f", "/dev/null"])
            .current_dir(&self.root)
            .env_clear()
            .env("HOME", &self.root)
            .env("USERPROFILE", &self.root)
            .env("SHELL", "/bin/sh")
            .env("TERM", "xterm-256color")
            .env("LANG", "C.UTF-8")
            .env("PATH", self.root.join("bin"))
            .env("TMPDIR", &self.root)
            .env("RODER_CONFIG_DIR", self.root.join("config"))
            .env("RODER_DATA_DIR", self.root.join("data"))
            .env("RODER_TUI_FIXTURE_ROOT", &self.root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }

    fn capture(&mut self) -> String {
        String::from_utf8_lossy(
            &self
                .tmux(&["capture-pane", "-p", "-t", "setup:0.0", "-S", "-200"])
                .stdout,
        )
        .into_owned()
    }

    fn wait_for(&mut self, needle: &str) -> String {
        loop {
            let text = self.capture();
            if text.contains(needle) {
                return text;
            }
            assert!(
                !self.root.join("exit-status").exists(),
                "Roder exited before {needle:?}; terminal:\n{text}"
            );
            assert!(
                Instant::now() < self.deadline,
                "timed out waiting for {needle:?}; terminal:\n{text}"
            );
            std::thread::sleep(Duration::from_millis(30));
        }
    }

    fn key(&mut self, key: &str) {
        self.tmux(&["send-keys", "-t", "setup:0.0", key]);
    }

    fn literal(&mut self, text: &str) {
        self.tmux(&["send-keys", "-l", "-t", "setup:0.0", text]);
    }

    fn wait_for_exit(&mut self) -> String {
        let status = self.root.join("exit-status");
        loop {
            if let Ok(status) = std::fs::read_to_string(&status)
                && !status.is_empty()
            {
                assert_eq!(status, "0", "terminal:\n{}", self.capture());
                return self.capture();
            }
            assert!(
                Instant::now() < self.deadline,
                "Roder did not exit; terminal:\n{}",
                self.capture()
            );
            std::thread::sleep(Duration::from_millis(30));
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
        // This unique -L name can only address this test's server, never a user's.
        if let Ok(mut child) = self.command().arg("kill-server").spawn() {
            let deadline = Instant::now() + Duration::from_secs(2);
            while child.try_wait().ok().flatten().is_none() {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            let _ = child.wait();
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn executable(name: &str) -> PathBuf {
    let path = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|directory| directory.join(name))
        .find(|path| {
            path.is_file()
                && std::fs::metadata(path)
                    .is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
        })
        .unwrap_or_else(|| panic!("{name} is required for RODER_TUI_E2E terminal tests"));
    std::fs::canonicalize(path).expect("resolve fixture dependency")
}

fn quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

#[test]
#[ignore = "requires tmux, python3, and explicit RODER_TUI_E2E=1"]
fn setup_returns_to_the_tui_after_offline_inherited_google_login() {
    let mut fixture = Fixture::new(true);
    fixture.start();
    fixture.wait_for("2. Connect your Google account");
    fixture.tmux(&["resize-window", "-t", "setup:0", "-x", "80", "-y", "24"]);
    let resized = fixture.wait_for("Enter: Google sign-in");
    assert!(
        resized.contains("2. Connect your Google account"),
        "{resized}"
    );
    assert!(!fixture.root.join("signed-in").exists());
    assert!(!fixture.root.join("login-started").exists());
    fixture.key("Enter");
    fixture.wait_for("OFFLINE LOGIN: enter the fixture-only code:");
    fixture.literal(FIXTURE_CODE);
    fixture.key("Enter");
    let ready = fixture.wait_for("Enter: continue to Roder");
    assert!(ready.contains("3. Ready"), "{ready}");
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("signed-in")).unwrap(),
        "offline-fixture-account",
    );
    fixture.key("Enter");
    let output = fixture.wait_for_exit();
    assert!(output.contains("Antigravity is ready."), "{output}");
    assert!(
        !output.contains("The cursor position could not be read"),
        "{output}"
    );
}

#[test]
#[ignore = "requires tmux, python3, and explicit RODER_TUI_E2E=1"]
fn escaping_missing_runtime_setup_does_not_install_or_create_config() {
    let mut fixture = Fixture::new(false);
    fixture.start();
    fixture.wait_for("Enter: install | Esc: not now");
    assert!(!fixture.root.join("config").exists());
    fixture.key("Escape");
    fixture.wait_for_exit();
    assert!(!fixture.root.join("config").exists());
    assert!(!fixture.root.join("data").exists());
    assert!(!fixture.root.join("login-started").exists());
    assert!(!fixture.root.join("signed-in").exists());
}

const FAKE_AGY: &str = r#"
import json, os, pathlib, sys, termios

root = pathlib.Path(os.environ["RODER_TUI_FIXTURE_ROOT"])
if sys.argv[1:] == ["--version"]:
    print("1.5.18", flush=True)
    sys.exit(0)
if sys.argv[1:] == ["acp", "--login"]:
    assert sys.stdin.isatty() and sys.stdout.isatty(), "login must inherit a terminal"
    flags = termios.tcgetattr(sys.stdin.fileno())[3]
    assert flags & termios.ICANON and flags & termios.ECHO, "login needs cooked input"
    (root / "login-started").write_text("offline")
    print("OFFLINE LOGIN: enter the fixture-only code:", flush=True)
    code = sys.stdin.readline().strip()
    if code != "RODER_OFFLINE_FIXTURE_CODE":
        print("Offline fixture received an incorrect code", flush=True)
        sys.exit(3)
    (root / "signed-in").write_text("offline-fixture-account")
    print("OFFLINE LOGIN COMPLETE", flush=True)
    sys.exit(0)

assert sys.argv[1] == "acp", sys.argv
for line in sys.stdin:
    request = json.loads(line)
    if "id" not in request:
        continue
    method = request.get("method")
    result = {}
    if method == "initialize":
        result = {"protocolVersion": 1, "agentCapabilities": {},
                  "authMethods": [{"id": "oauth-personal", "name": "Offline Google"}]}
    elif method == "authenticate" and not (root / "signed-in").exists():
        print(json.dumps({"jsonrpc": "2.0", "id": request["id"],
                          "error": {"code": -32000, "message": "Offline sign-in required"}}),
              flush=True)
        continue
    elif method == "session/new":
        assert (root / "signed-in").exists()
        result = {"sessionId": "offline-setup",
                  "models": {"currentModelId": "gemini-offline",
                             "availableModels": [{"modelId": "gemini-offline",
                                                  "name": "Offline Gemini"}]}}
    print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}), flush=True)
"#;
