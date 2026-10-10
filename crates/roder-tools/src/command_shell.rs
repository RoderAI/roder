use roder_api::tools::ToolExecutionContext;

pub(crate) fn shell_for_context(ctx: &ToolExecutionContext, fallback: &str) -> String {
    ctx.command_shell
        .as_deref()
        .and_then(roder_api::command_shell::normalize_command_shell)
        .unwrap_or_else(|| fallback.to_string())
}

pub(crate) fn command_args_for_shell(shell: &str, command: &str, login: bool) -> Vec<String> {
    if is_powershell(shell) {
        let mut args = vec!["-NoProfile".to_string()];
        if cfg!(windows) {
            args.extend(["-ExecutionPolicy".to_string(), "Bypass".to_string()]);
        }
        args.extend(["-Command".to_string(), command.to_string()]);
        return args;
    }

    vec![
        if login { "-lc" } else { "-c" }.to_string(),
        command.to_string(),
    ]
}

/// Starts the child in its own session so it has no controlling terminal.
///
/// Tool subprocesses otherwise inherit roder's session and with it the TUI's
/// terminal. An interactive shell started by a tool (`zsh -ic ...`) opens
/// `/dev/tty` and makes its own process group the terminal's foreground
/// group. Roder is then a background group, and its next read from the
/// terminal stops the whole TUI with SIGTTIN. Without a controlling terminal
/// those shells (and `sudo`, `ssh`, ...) fall back to non-interactive behaviour
/// instead of taking over the user's screen.
pub(crate) fn detach_controlling_terminal(command: &mut tokio::process::Command) {
    #[cfg(unix)]
    // SAFETY: the closure runs between fork and exec and only calls
    // async-signal-safe `setsid`.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    #[cfg(not(unix))]
    let _ = command;
}

/// SIGKILLs the whole process group led by a child that went through
/// [`detach_controlling_terminal`].
///
/// Detached children no longer share roder's process group, so killing only the
/// shell (`Child::kill`, `kill_on_drop`) would leave everything it started
/// running after a timeout or cancellation.
pub(crate) fn kill_process_group(pid: u32) {
    #[cfg(unix)]
    // SAFETY: plain syscall; a stale or foreign id only yields ESRCH/EPERM.
    unsafe {
        libc::killpg(pid as libc::pid_t, libc::SIGKILL);
    }
    #[cfg(not(unix))]
    let _ = pid;
}

/// Kills the child's process group when dropped, unless disarmed.
///
/// Arm it for the lifetime of a command and disarm after the command finished
/// on its own: a timeout or a cancelled turn then takes the descendants down,
/// while background processes a finished command left behind keep running.
pub(crate) struct ProcessGroupKillOnDrop {
    pid: Option<u32>,
}

impl ProcessGroupKillOnDrop {
    pub(crate) fn new(pid: Option<u32>) -> Self {
        Self { pid }
    }

    pub(crate) fn disarm(&mut self) {
        self.pid = None;
    }
}

impl Drop for ProcessGroupKillOnDrop {
    fn drop(&mut self) {
        if let Some(pid) = self.pid.take() {
            kill_process_group(pid);
        }
    }
}

fn is_powershell(shell: &str) -> bool {
    let name = shell
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(shell)
        .trim()
        .trim_end_matches(".exe")
        .to_ascii_lowercase();
    matches!(name.as_str(), "powershell" | "pwsh")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Returns `(pid, pgid)` as seen by a `sh` child spawned with `command`.
    #[cfg(unix)]
    async fn child_pid_and_pgid(mut command: tokio::process::Command) -> (i64, i64) {
        let output = command
            .args(["-c", "ps -o pid=,pgid= -p $$"])
            .output()
            .await
            .expect("spawn sh");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut fields = stdout
            .split_whitespace()
            .map(|field| field.parse::<i64>().expect("numeric ps field"));
        (fields.next().expect("pid"), fields.next().expect("pgid"))
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn detached_child_leads_its_own_session() {
        let mut command = tokio::process::Command::new("sh");
        detach_controlling_terminal(&mut command);

        let (pid, pgid) = child_pid_and_pgid(command).await;

        // `setsid` makes the child a session and process-group leader, which
        // is what drops the inherited controlling terminal.
        assert_eq!(pid, pgid);
    }

    /// A killed orphan can linger as a zombie until init reaps it; that is dead.
    #[cfg(unix)]
    fn process_alive(pid: i32) -> bool {
        // SAFETY: signal 0 only probes for existence.
        if unsafe { libc::kill(pid, 0) } != 0 {
            return false;
        }
        let state = std::process::Command::new("ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output()
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
            .unwrap_or_default();
        !state.is_empty() && !state.starts_with('Z')
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn dropping_the_guard_kills_descendants_of_a_detached_child() {
        let dir = std::env::temp_dir().join(format!("roder-pgkill-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pidfile = dir.join("grandchild.pid");
        let _ = std::fs::remove_file(&pidfile);

        let mut command = tokio::process::Command::new("sh");
        command
            .arg("-c")
            .arg(format!("sleep 60 & echo $! > {}; wait", pidfile.display()))
            .kill_on_drop(true);
        detach_controlling_terminal(&mut command);
        let child = command.spawn().expect("spawn sh");
        let guard = ProcessGroupKillOnDrop::new(child.id());

        let mut grandchild = None;
        for _ in 0..250 {
            if let Some(pid) = std::fs::read_to_string(&pidfile)
                .ok()
                .and_then(|text| text.trim().parse::<i32>().ok())
            {
                grandchild = Some(pid);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let grandchild = grandchild.expect("grandchild pid was written");
        assert!(process_alive(grandchild));

        drop(guard);

        let mut gone = false;
        for _ in 0..250 {
            if !process_alive(grandchild) {
                gone = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        drop(child);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(gone, "descendant {grandchild} survived the group kill");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn undetached_child_shares_the_parent_process_group() {
        let (pid, pgid) = child_pid_and_pgid(tokio::process::Command::new("sh")).await;

        assert_ne!(pid, pgid);
    }

    #[test]
    fn powershell_uses_command_argument_instead_of_unix_shell_flags() {
        let args = command_args_for_shell("powershell", "pnpm test", true);

        assert!(args.contains(&"-Command".to_string()));
        assert!(args.contains(&"pnpm test".to_string()));
        assert!(!args.contains(&"-lc".to_string()));
        assert!(!args.contains(&"-c".to_string()));
    }

    #[test]
    fn pwsh_path_is_detected_as_powershell() {
        let args = command_args_for_shell(r"C:\Program Files\PowerShell\7\pwsh.exe", "dir", false);

        assert!(args.contains(&"-Command".to_string()));
        assert!(!args.contains(&"-c".to_string()));
    }

    #[test]
    fn unix_shells_keep_login_and_non_login_flags() {
        assert_eq!(
            command_args_for_shell("bash", "printf ok", true),
            vec!["-lc".to_string(), "printf ok".to_string()]
        );
        assert_eq!(
            command_args_for_shell("/bin/sh", "printf ok", false),
            vec!["-c".to_string(), "printf ok".to_string()]
        );
    }
}
