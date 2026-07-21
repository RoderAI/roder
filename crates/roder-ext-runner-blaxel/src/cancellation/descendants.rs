use std::time::Duration;

use crate::client::BlaxelClient;

use super::COMMAND_TAG_ENV;

const DESCENDANT_CLEANUP_LEASE_SECONDS: u64 = 15;
const DESCENDANT_CLEANUP_TIMEOUT: Duration = Duration::from_secs(20);

pub(super) async fn cleanup_tagged_descendants(
    client: &BlaxelClient,
    endpoint: &str,
    command_tag: &str,
) -> bool {
    let cleanup_name = format!("roder-cleanup-{}", uuid::Uuid::new_v4().simple());
    let command = descendant_cleanup_command(command_tag);
    matches!(
        tokio::time::timeout(
            DESCENDANT_CLEANUP_TIMEOUT,
            client.exec(
                endpoint,
                &cleanup_name,
                &command,
                None,
                &[],
                DESCENDANT_CLEANUP_LEASE_SECONDS,
            ),
        )
        .await,
        Ok(Ok(process)) if process.exit_code == Some(0)
    )
}

fn descendant_cleanup_command(command_tag: &str) -> String {
    let exact_tag = format!("{COMMAND_TAG_ENV}={command_tag}");
    let script = format!(
        "command -v python3 >/dev/null 2>&1 || exit 125; exec python3 -c {} {}",
        shell_quote(DESCENDANT_CLEANUP_PYTHON),
        shell_quote(&exact_tag)
    );
    format!("/bin/sh -c {}", shell_quote(&script))
}

const DESCENDANT_CLEANUP_PYTHON: &str = r#"import errno
import os
import signal
import sys
import time

if not hasattr(os, "pidfd_open") or not hasattr(signal, "pidfd_send_signal"):
    raise SystemExit(125)

tag = sys.argv[1].encode()
vanished = {errno.ENOENT, errno.ESRCH}
pf_kthread = 0x00200000


class PidfdUnavailable(Exception):
    pass


def pidfd_open(pid):
    try:
        return os.pidfd_open(pid, 0)
    except OSError as error:
        if error.errno in vanished:
            return None
        if error.errno == errno.ENOSYS:
            raise PidfdUnavailable from error
        raise


def pidfd_signal(pidfd, sig):
    try:
        signal.pidfd_send_signal(pidfd, sig, None, 0)
        return True
    except ProcessLookupError:
        return False
    except OSError as error:
        if error.errno == errno.ENOSYS:
            raise PidfdUnavailable from error
        raise


def kernel_thread(pid, pidfd):
    try:
        with open(f"/proc/{pid}/stat", "rb", buffering=0) as stat_file:
            stat = stat_file.read()
    except OSError:
        if not pidfd_signal(pidfd, 0):
            return None
        raise

    # Confirm the pidfd target is still alive after the numeric /proc read. If
    # it exited, the path could already refer to a recycled PID and this scan
    # must make no claim about either generation.
    if not pidfd_signal(pidfd, 0):
        return None
    separator = stat.rfind(b") ")
    fields = stat[separator + 2:].split() if separator >= 0 else []
    if len(fields) <= 6:
        raise OSError(errno.EIO, "invalid /proc stat record")
    try:
        flags = int(fields[6])
    except ValueError as error:
        raise OSError(errno.EIO, "invalid /proc flags") from error
    return bool(flags & pf_kthread)


def read_environ(pid, pidfd):
    try:
        with open(f"/proc/{pid}/environ", "rb", buffering=0) as environ:
            return environ.read()
    except FileNotFoundError:
        return None
    except OSError:
        if not pidfd_signal(pidfd, 0):
            return None
        # Linux kernel threads have no userspace environment and commonly
        # report ESRCH for /proc/<pid>/environ despite the pidfd remaining
        # live. Skip only a positively identified PF_KTHREAD task; every other
        # inaccessible extant process still fails closed.
        if kernel_thread(pid, pidfd) is not False:
            return None
        raise


def scan_and_signal(sig):
    found = False
    with os.scandir("/proc") as entries:
        for entry in entries:
            if not entry.name.isdigit():
                continue
            pidfd = pidfd_open(int(entry.name))
            if pidfd is None:
                continue
            try:
                environ = read_environ(entry.name, pidfd)
                if environ is None or tag not in environ.split(b"\0"):
                    continue
                found = True
                if sig is not None:
                    pidfd_signal(pidfd, sig)
            finally:
                os.close(pidfd)
    return found


def reap(sig, attempts):
    quiet = 0
    for _ in range(attempts):
        if scan_and_signal(sig):
            quiet = 0
        else:
            quiet += 1
            if quiet >= 2:
                return True
        time.sleep(1)
    return False


try:
    clean = reap(signal.SIGTERM, 2) or reap(signal.SIGKILL, 5)
except PidfdUnavailable:
    raise SystemExit(125)
except OSError as error:
    print(f"roder descendant cleanup failed: errno={error.errno}", file=sys.stderr)
    raise SystemExit(1)

raise SystemExit(0 if clean else 1)
"#;

pub(crate) fn shell_quote(value: &str) -> String {
    if !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'/' | b':' | b'=')
        })
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_window_exceeds_remote_lease() {
        assert!(DESCENDANT_CLEANUP_TIMEOUT > Duration::from_secs(DESCENDANT_CLEANUP_LEASE_SECONDS));
    }

    #[test]
    fn descendant_cleanup_uses_exact_environment_entries_and_pidfds() {
        let command = descendant_cleanup_command("tag-with-'quote");

        assert!(command.contains("RODER_BLAXEL_COMMAND_TAG=tag-with-"));
        assert!(DESCENDANT_CLEANUP_PYTHON.contains("os.pidfd_open(pid, 0)"));
        assert!(DESCENDANT_CLEANUP_PYTHON.contains("signal.pidfd_send_signal"));
        assert!(DESCENDANT_CLEANUP_PYTHON.contains("environ.split(b\"\\0\")"));
        assert!(DESCENDANT_CLEANUP_PYTHON.contains("pf_kthread = 0x00200000"));
        assert!(DESCENDANT_CLEANUP_PYTHON.contains("kernel_thread(pid, pidfd) is not False"));
        assert!(DESCENDANT_CLEANUP_PYTHON.contains("reap(signal.SIGTERM, 2)"));
        assert!(DESCENDANT_CLEANUP_PYTHON.contains("reap(signal.SIGKILL, 5)"));
    }
}
