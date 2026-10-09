use crate::{CuaConfig, CuaTarget, CuaTransport, DriverReply};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::{Mutex, OwnedMutexGuard, oneshot};

static DESKTOP_GATE: OnceLock<Arc<Mutex<()>>> = OnceLock::new();
static DESKTOP_UNCERTAIN: AtomicBool = AtomicBool::new(false);
static DESKTOP_GENERATION: AtomicU64 = AtomicU64::new(0);

/// An opaque process-wide fence for the shared physical macOS desktop.
/// The transport worker keeps a reference after its caller is cancelled.
pub struct LocalDesktopLease {
    _guard: OwnedMutexGuard<()>,
    dispatch: Arc<Mutex<()>>,
}
impl LocalDesktopLease {
    pub(crate) async fn acquire() -> Arc<Self> {
        Arc::new(Self {
            dispatch: Arc::new(Mutex::new(())),
            _guard: DESKTOP_GATE
                .get_or_init(|| Arc::new(Mutex::new(())))
                .clone()
                .lock_owned()
                .await,
        })
    }
    pub(crate) fn generation(&self) -> u64 {
        DESKTOP_GENERATION.load(Ordering::SeqCst)
    }
    pub(crate) fn begin_input(&self) {
        DESKTOP_GENERATION.fetch_add(1, Ordering::SeqCst);
    }
}

pub struct LocalMacosTransport {
    config: CuaConfig,
}
impl LocalMacosTransport {
    pub fn new(config: CuaConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl CuaTransport for LocalMacosTransport {
    async fn call(
        &self,
        target: CuaTarget<'_>,
        tool: &str,
        arguments: Value,
        timeout_ms: u64,
    ) -> anyhow::Result<DriverReply> {
        anyhow::ensure!(cfg!(target_os = "macos"), "local-macos requires macOS");
        let CuaTarget::LocalMacos(lease) = target else {
            anyhow::bail!("local macOS Cua transport cannot operate a remote runner");
        };
        anyhow::ensure!(
            !DESKTOP_UNCERTAIN.load(Ordering::SeqCst),
            "local Cua dispatch remains uncertain; restart CuaDriver.app and Roder before input"
        );
        // An after-action capture must wait for a timed-out input worker too.
        let dispatch = tokio::time::timeout(
            Duration::from_millis(timeout_ms),
            lease.dispatch.clone().lock_owned(),
        )
        .await
        .map_err(|_| {
            anyhow::anyhow!("local Cua input is still finishing; no new call dispatched")
        })?;
        anyhow::ensure!(
            !DESKTOP_UNCERTAIN.load(Ordering::SeqCst),
            "local Cua dispatch remains uncertain; restart CuaDriver.app and Roder before input"
        );
        let config = self.config.clone();
        let tool = tool.to_owned();
        let lease = lease.clone();
        let (sender, receiver) = oneshot::channel();
        tokio::spawn(async move {
            let result = worker(&config, &tool, arguments, timeout_ms, &sender).await;
            let _ = sender.send(result);
            drop(dispatch);
            drop(lease);
        });
        tokio::time::timeout(Duration::from_millis(timeout_ms), receiver)
            .await
            .map_err(|_| anyhow::anyhow!("Cua call timed out; input may still be finishing under the desktop fence. Observe before another action."))?
            .map_err(|_| anyhow::anyhow!("Cua local worker stopped; observe before another action"))?
    }
}

#[cfg(unix)]
fn verify_daemon_generation(socket: &str, input: bool, before: u64) -> anyhow::Result<()> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    type Identity = (String, u64, u64, i64, i64);
    static IDENTITY: std::sync::Mutex<Option<Identity>> = std::sync::Mutex::new(None);
    let metadata = std::fs::metadata(socket).map_err(|_| {
        anyhow::anyhow!("Cua daemon socket unavailable; launch the signed CuaDriver.app")
    })?;
    anyhow::ensure!(
        metadata.file_type().is_socket(),
        "Cua daemon path is not a Unix socket"
    );
    let current = (
        socket.to_owned(),
        metadata.dev(),
        metadata.ino(),
        metadata.ctime(),
        metadata.ctime_nsec(),
    );
    let mut identity = IDENTITY.lock().unwrap();
    if identity.as_ref() != Some(&current) {
        *identity = Some(current);
        DESKTOP_GENERATION.fetch_add(1, Ordering::SeqCst);
    }
    anyhow::ensure!(
        !input || before == DESKTOP_GENERATION.load(Ordering::SeqCst),
        "Cua daemon changed; observe again before input"
    );
    Ok(())
}

async fn worker(
    config: &CuaConfig,
    tool: &str,
    arguments: Value,
    timeout_ms: u64,
    sender: &oneshot::Sender<anyhow::Result<DriverReply>>,
) -> anyhow::Result<DriverReply> {
    let program = config.program();
    #[cfg(unix)]
    let generation = DESKTOP_GENERATION.load(Ordering::SeqCst);
    let version = command(program, &["--version".into()], 5_000, 1024).await?;
    anyhow::ensure!(
        version.0
            && std::str::from_utf8(&version.1)?.split_whitespace().last()
                == Some(crate::DRIVER_VERSION),
        "unsupported local Cua Driver version; expected {}",
        crate::DRIVER_VERSION
    );
    // Do not dispatch a queued command after its Roder caller disappeared.
    if sender.is_closed() {
        anyhow::bail!("Cua caller cancelled before dispatch");
    }
    let socket = config.local_socket_path()?;
    #[cfg(unix)]
    verify_daemon_generation(&socket, crate::specs::is_input(tool), generation)?;
    let args = vec![
        "call".into(),
        tool.into(),
        arguments.to_string(),
        "--socket".into(),
        socket,
    ];
    // Native atomic drag gestures are bounded to 5s. Keep the client/fence alive
    // for at least 10s even when the caller has a shorter deadline. The signed
    // app-owned daemon remains responsible for TCC and native input.
    let response = command(program, &args, timeout_ms.max(10_000), 12 * 1024 * 1024).await;
    let result = response.and_then(|(success, bytes)| {
        let observation: Value = match serde_json::from_slice(&bytes) {
            Ok(value) => value,
            Err(_) => {
                // The pinned CLI prints a tool's text content when there is
                // no structuredContent (including explicit AX refusals).
                let text = std::str::from_utf8(&bytes)?.trim();
                anyhow::ensure!(
                    !text.is_empty()
                        && text.len() <= 8192
                        && !text.starts_with('{')
                        && !text.starts_with('['),
                    "Cua daemon returned no valid observation; no input was retried"
                );
                serde_json::json!({"summary":text,"effect":"unverifiable"})
            }
        };
        anyhow::ensure!(observation.is_object(), "Cua observation must be an object");
        Ok((success, observation))
    });
    let (success, observation) = match result {
        Ok(result) => result,
        Err(error) => {
            // A lost/invalid reply cannot prove native work has stopped. Fail
            // closed across sessions rather than releasing another input.
            DESKTOP_UNCERTAIN.store(true, Ordering::SeqCst);
            return Err(error);
        }
    };
    Ok(DriverReply {
        observation,
        is_error: !success,
    })
}

async fn command(
    program: &str,
    args: &[String],
    timeout: u64,
    limit: u64,
) -> anyhow::Result<(bool, Vec<u8>)> {
    let mut command = Command::new(program);
    command
        .args(args)
        .env_clear()
        .kill_on_drop(true)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    for name in ["HOME", "PATH", "TMPDIR", "LANG"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command.env("CUA_DRIVER_RS_TELEMETRY_ENABLED", "0");
    let mut child = command.spawn()?;
    let mut stdout = child.stdout.take().unwrap().take(limit + 1);
    let mut stderr = child.stderr.take().unwrap().take(8193);
    let mut output = Vec::new();
    let mut errors = Vec::new();
    let operation = async {
        let (_, _, status) = tokio::try_join!(
            stdout.read_to_end(&mut output),
            stderr.read_to_end(&mut errors),
            child.wait(),
        )?;
        anyhow::ensure!(
            output.len() as u64 <= limit && errors.len() <= 8192,
            "Cua client output exceeded its bound"
        );
        Ok((status.success(), output))
    };
    tokio::time::timeout(Duration::from_millis(timeout), operation)
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "Cua client did not finish within its worker deadline; input was not retried"
            )
        })?
}

#[cfg(all(test, target_os = "macos"))]
#[path = "local_tests.rs"]
mod tests;
