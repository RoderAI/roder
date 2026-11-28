//! Starting child processes.

use std::io;
use std::time::Duration;

use tokio::process::{Child, Command};

/// How often a spawn that hits "Text file busy" is retried.
const BUSY_RETRIES: u32 = 10;

/// Spawns `command`, retrying briefly when the program is still open for
/// writing ("Text file busy"). That happens when the executable was just
/// written or replaced, such as an updated CLI or a script a test wrote, and
/// another thread forked while its write handle was still open.
pub async fn spawn_retrying_busy(command: &mut Command) -> io::Result<Child> {
    let mut attempt = 0;
    loop {
        match command.spawn() {
            Err(err)
                if err.kind() == io::ErrorKind::ExecutableFileBusy && attempt < BUSY_RETRIES =>
            {
                attempt += 1;
                tokio::time::sleep(Duration::from_millis(10 * u64::from(attempt))).await;
            }
            result => return result,
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn spawns_a_program() {
        let mut command = Command::new("true");
        let status = spawn_retrying_busy(&mut command)
            .await
            .unwrap()
            .wait()
            .await
            .unwrap();
        assert!(status.success());
    }

    #[tokio::test]
    async fn reports_other_errors_without_retrying() {
        let mut command = Command::new("/nonexistent/roder-spawn-test");
        let err = spawn_retrying_busy(&mut command).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }
}
