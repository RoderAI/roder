use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt};

use super::{ExecExit, ExecSession, MAX_BUFFER_BYTES, exit_from_status};
use crate::command_shell::{ProcessGroupKillOnDrop, kill_process_group};
use crate::exec_output::trim_output_buffer_to_max_bytes;

pub(super) fn spawn_output_reader<R>(mut reader: R, session: Arc<ExecSession>)
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut buf = [0_u8; 8192];
        loop {
            match reader.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let mut output = session.output.lock().await;
                    output.push_str(&String::from_utf8_lossy(&buf[..n]));
                    let mut cursor = session.cursor.lock().await;
                    trim_output_buffer_to_max_bytes(&mut output, &mut cursor, MAX_BUFFER_BYTES);
                }
            }
        }
    });
}

pub(super) fn spawn_waiter(
    mut child: tokio::process::Child,
    session: Arc<ExecSession>,
    timeout_ms: Option<u64>,
) {
    // The waiter outlives a yielded tool call. Keep group custody in its
    // future so runtime shutdown also cancels detached command descendants.
    let mut kill_group = ProcessGroupKillOnDrop::new(child.id());
    tokio::spawn(async move {
        let exit = if let Some(timeout_ms) = timeout_ms {
            tokio::select! {
                status = child.wait() => completed_exit(status, &mut kill_group),
                _ = tokio::time::sleep(Duration::from_millis(timeout_ms)) => {
                    // The command may have exited just before the deadline and
                    // lost the select race; keep its real status and leave any
                    // background processes it started alone.
                    match child.try_wait() {
                        Ok(Some(status)) => completed_exit(Ok(status), &mut kill_group),
                        _ => {
                            if let Some(pid) = child.id() {
                                kill_process_group(pid);
                            }
                            let _ = child.kill().await;
                            let _ = child.wait().await;
                            ExecExit { exit_code: -1, timed_out: true }
                        }
                    }
                }
            }
        } else {
            completed_exit(child.wait().await, &mut kill_group)
        };
        *session.exit.lock().await = Some(exit);
        session.exit_notify.notify_waiters();
    });
}

fn completed_exit(
    status: std::io::Result<std::process::ExitStatus>,
    kill_group: &mut ProcessGroupKillOnDrop,
) -> ExecExit {
    if status.is_ok() {
        // A command that finishes normally may intentionally leave a server
        // running. Only interrupted/unfinished command groups belong to cleanup.
        kill_group.disarm();
    }
    exit_from_status(status, false)
}

#[cfg(all(test, unix))]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    use super::super::{ExecSessionManager, SpawnOptions, spawn_exec_session};
    use crate::command_shell::ProcessGroupKillOnDrop;

    struct Fixture {
        root: PathBuf,
        group: Option<ProcessGroupKillOnDrop>,
    }

    impl Fixture {
        fn new() -> Self {
            static NEXT_ID: AtomicU64 = AtomicU64::new(0);
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "roder-exec-owner-{}-{nonce}-{id}",
                std::process::id()
            ));
            std::fs::create_dir(&root).unwrap();
            Self { root, group: None }
        }

        fn command(&self, wait: bool) -> String {
            format!(
                "sleep 60 & echo $! > '{}'; echo $$ > '{}'; {}",
                self.root.join("child.pid").display(),
                self.root.join("group.pid").display(),
                if wait { "wait" } else { "exit 0" }
            )
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            self.group.take();
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    async fn read_pid(path: &Path) -> u32 {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Ok(text) = std::fs::read_to_string(path)
                    && let Ok(pid) = text.trim().parse::<u32>()
                {
                    return pid;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("owned command should publish its PID")
    }

    fn alive(pid: u32) -> bool {
        let output = std::process::Command::new("ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output()
            .unwrap();
        let state = String::from_utf8_lossy(&output.stdout);
        let state = state.trim();
        !state.is_empty() && !state.starts_with('Z')
    }

    fn start(wait: bool) -> (tokio::runtime::Runtime, Fixture, u32) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let mut fixture = Fixture::new();
        let (pid, group) = runtime.block_on(async {
            let (_, session) = spawn_exec_session(
                &ExecSessionManager::default(),
                SpawnOptions {
                    command: fixture.command(wait),
                    cwd_path: fixture.root.clone(),
                    cwd: fixture.root.display().to_string(),
                    shell: "/bin/sh".to_string(),
                    login: false,
                    tty: false,
                    timeout_ms: None,
                },
            )
            .await
            .unwrap();
            let pid = read_pid(&fixture.root.join("child.pid")).await;
            let group = read_pid(&fixture.root.join("group.pid")).await;
            if !wait {
                tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    while session.exit.lock().await.is_none() {
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    }
                })
                .await
                .unwrap();
            }
            (pid, group)
        });
        fixture.group = Some(ProcessGroupKillOnDrop::new(Some(group)));
        (runtime, fixture, pid)
    }

    #[test]
    fn runtime_shutdown_kills_an_active_exec_command_and_its_descendants() {
        let (runtime, _fixture, pid) = start(true);
        assert!(alive(pid));
        drop(runtime);
        let deadline = Instant::now() + std::time::Duration::from_secs(3);
        while alive(pid) && Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            !alive(pid),
            "runtime shutdown left an owned exec descendant running"
        );
    }

    #[test]
    fn normal_command_completion_preserves_its_background_process() {
        let (runtime, _fixture, pid) = start(false);
        assert!(alive(pid));
        drop(runtime);
        assert!(
            alive(pid),
            "a completed command's background process must not be cancelled"
        );
    }
}
