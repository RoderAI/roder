#[cfg(test)]
extern crate self as roder_ext_runner_blaxel;

use std::sync::Arc;

use roder_api::capabilities::CapabilityRequest;
use roder_api::extension::{
    ExtensionManifest, ExtensionRegistryBuilder, ProvidedService, RoderExtension,
};
use roder_api::remote_runner::{RemoteRunnerProvider, RunnerDestination};
use semver::Version;

mod cancellation;
mod client;
pub mod config;
mod egress;
pub mod forks;
mod ports;
mod provider;
pub use provider::BlaxelRunnerProvider;
mod session;
mod standby;
mod workspace;
pub use workspace::BlaxelWorkspace;

pub use client::{BlaxelClient, ProcessResponse, Sandbox, wait_until_ready};
pub use config::{
    BASE_URL_ENV, BlaxelConfig, CleanupMode, DEFAULT_BASE_URL, EXTENSION_ID, ExpirationPolicy,
    ExpirationPolicyType, LIVE_ENV, MAX_STANDBY_AFTER_SECONDS, PROVIDER_ID, Redacted,
    SandboxLifecycle, TOKEN_ENV, WORKSPACE_ENV,
};
pub use ports::{PortProtocol, SandboxPort};
pub use session::BlaxelRunnerSession;

#[derive(Debug, Default)]
pub struct BlaxelRunnerExtension;

impl RoderExtension for BlaxelRunnerExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: EXTENSION_ID.to_string(),
            name: "Blaxel Runner".to_string(),
            version: Version::new(0, 1, 0),
            api_version: "0.1.0".to_string(),
            description: Some(
                "Runs remote-runner sessions inside Blaxel sandboxes with pause, resume, detach, \
                 and rejoin support."
                    .to_string(),
            ),
            provides: vec![ProvidedService::RemoteRunnerProvider(
                PROVIDER_ID.to_string(),
            )],
            required_capabilities: vec![
                CapabilityRequest::new("network.http"),
                CapabilityRequest::new(format!("secret.read.{TOKEN_ENV}")),
            ],
        }
    }

    fn install(&self, registry: &mut ExtensionRegistryBuilder) -> anyhow::Result<()> {
        registry.remote_runner_provider(Arc::new(BlaxelRunnerProvider::default()));
        Ok(())
    }
}

/// User-Agent sent on every Blaxel request. Blaxel's edge (CloudFront) rejects
/// requests with an empty User-Agent (403), so this must be non-empty.
const USER_AGENT: &str = concat!("roder-ext-runner-blaxel/", env!("CARGO_PKG_VERSION"));

/// Sanitize an arbitrary id into a Blaxel sandbox name: lowercase alphanumeric
/// with single hyphens, max 49 chars (per the control-plane name rule).
pub fn sanitize_name(raw: &str) -> String {
    let mut out = String::new();
    let mut last_hyphen = false;
    for ch in raw.to_ascii_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_hyphen = false;
        } else if !last_hyphen && !out.is_empty() {
            out.push('-');
            last_hyphen = true;
        }
    }
    let trimmed = out.trim_matches('-');
    let truncated: String = trimmed.chars().take(49).collect();
    let name = truncated.trim_end_matches('-').to_string();
    if name.is_empty() {
        "roder-sandbox".to_string()
    } else {
        name
    }
}

/// Opt-in live smoke (`RODER_LIVE_BLAXEL_RUNNER=1`) exercising create -> exec ->
/// pause -> resume -> detach -> rejoin -> delete against a real Blaxel account.
pub async fn run_live_smoke_if_enabled() -> anyhow::Result<()> {
    if std::env::var(LIVE_ENV).ok().as_deref() != Some("1") {
        eprintln!("set {LIVE_ENV}=1 to run the live blaxel runner smoke");
        return Ok(());
    }
    run_live_smoke().await
}

async fn run_live_smoke() -> anyhow::Result<()> {
    use roder_api::remote_runner::{
        RunnerCommandRequest, RunnerFileReadRequest, RunnerFileWriteRequest, RunnerManifest,
        RunnerPortRequest,
    };

    let provider = BlaxelRunnerProvider::default();
    let destination = RunnerDestination {
        id: "blaxel-live".to_string(),
        provider_id: PROVIDER_ID.to_string(),
        config: serde_json::json!({
            "sandbox_name_prefix": "roder-live",
            "working_dir": "/home/user/roder-live",
            "cleanup": "delete-on-close"
        }),
        default_manifest: RunnerManifest::default(),
    };
    let session = provider.create_session(destination).await?;

    // Command exec.
    let out = session
        .run_command(RunnerCommandRequest {
            command_id: "smoke".to_string(),
            program: "echo".to_string(),
            args: vec!["blaxel-live".to_string()],
            cwd: None,
            env: Vec::new(),
            timeout_ms: None,
        })
        .await?;
    println!("live exec stdout: {}", out.stdout.trim());
    anyhow::ensure!(out.stdout.trim() == "blaxel-live", "unexpected exec stdout");

    // The child creates a new session and ignores cooperative signals. Blaxel's
    // named-process DELETE kills its supervisor but must not leave this child
    // alive long enough to perform the delayed write.
    let marker_id = uuid::Uuid::new_v4().simple().to_string();
    let ready_marker = format!("cancel-ready-{marker_id}.txt");
    let cancelled_marker = format!("cancelled-marker-{marker_id}.txt");
    let marker_command = format!(
        "setsid sh -c 'trap \"\" TERM INT HUP; set -f; \
         stat=$(cat /proc/$$/stat); fields=${{stat##*) }}; set -- $fields; \
         test \"$#\" -ge 20 || exit 125; shift 19; start_time=$1; \
         printf \"%s %s\" \"$$\" \"$start_time\" > {ready_marker}; \
         sleep 5; printf ghost > {cancelled_marker}' >/dev/null 2>&1 & wait"
    );
    let cancellation_session = session.clone();
    let cancelled = tokio::spawn(async move {
        cancellation_session
            .run_command(RunnerCommandRequest {
                command_id: "smoke-cancel".to_string(),
                program: "sh".to_string(),
                args: vec!["-lc".to_string(), marker_command],
                cwd: None,
                env: Vec::new(),
                timeout_ms: Some(10_000),
            })
            .await
    });
    let ready = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if let Ok(ready) = session
                .read_file(RunnerFileReadRequest {
                    path: ready_marker.clone().into(),
                })
                .await
            {
                break ready;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("detached cancellation child did not become ready"))?;
    let child_identity = String::from_utf8(ready.contents)
        .map_err(|error| anyhow::anyhow!("detached child wrote an invalid identity: {error}"))?;
    let mut identity_parts = child_identity.split_whitespace();
    let child_pid = identity_parts.next().unwrap_or_default();
    let child_start_time = identity_parts.next().unwrap_or_default();
    anyhow::ensure!(
        !child_pid.is_empty()
            && child_pid.bytes().all(|byte| byte.is_ascii_digit())
            && !child_start_time.is_empty()
            && child_start_time.bytes().all(|byte| byte.is_ascii_digit())
            && identity_parts.next().is_none(),
        "detached child wrote an invalid pid/start-time identity: {child_identity:?}"
    );
    let leak_deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(5_500);
    let mut killed = false;
    for _ in 0..20 {
        if session.cancel_command(&"smoke-cancel".to_string()).await? {
            killed = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    anyhow::ensure!(killed, "live Blaxel command was not cancellable");
    let cancelled_output = cancelled.await??;
    anyhow::ensure!(
        cancelled_output.exit_code != Some(0),
        "cancelled command unexpectedly completed successfully"
    );
    let pid_probe = session
        .run_command(RunnerCommandRequest {
            command_id: "smoke-cancel-pid-probe".to_string(),
            program: "sh".to_string(),
            args: vec![
                "-c".to_string(),
                format!(
                    "set -f; stat=$(cat /proc/{child_pid}/stat 2>/dev/null) || exit 1; \
                     fields=${{stat##*) }}; set -- $fields; test \"$#\" -ge 20 || exit 1; \
                     shift 19; test \"$1\" = {child_start_time}"
                ),
            ],
            cwd: None,
            env: Vec::new(),
            timeout_ms: Some(2_000),
        })
        .await?;
    anyhow::ensure!(
        pid_probe.exit_code.is_some_and(|exit_code| exit_code != 0),
        "detached child pid {child_pid} with start time {child_start_time} survived cancellation"
    );
    tokio::time::sleep_until(leak_deadline).await;
    anyhow::ensure!(
        session
            .read_file(RunnerFileReadRequest {
                path: cancelled_marker.into(),
            })
            .await
            .is_err(),
        "cancelled command mutated the workspace after interruption"
    );
    println!("live cancellation: tagged descendants reaped, delayed marker absent");

    // File write then read round-trips through the per-sandbox filesystem API.
    session
        .write_file(RunnerFileWriteRequest {
            path: "hello.txt".into(),
            contents: b"hello blaxel".to_vec(),
        })
        .await?;
    let file = session
        .read_file(RunnerFileReadRequest {
            path: "hello.txt".into(),
        })
        .await?;
    anyhow::ensure!(file.contents == b"hello blaxel", "file round-trip mismatch");
    println!(
        "live file read: {}",
        String::from_utf8_lossy(&file.contents)
    );

    // Port preview returns a public preview URL.
    let preview = session
        .expose_port(RunnerPortRequest {
            port: 3000,
            label: Some("web".to_string()),
        })
        .await?;
    println!("live preview url: {:?}", preview.url);

    // Lifecycle: pause -> resume -> detach -> rejoin -> delete.
    session.pause().await?;
    session.resume().await?;
    let state = session.detach().await?;
    let rejoined = provider.rejoin_session(state).await?;
    // Prove rejoin reuses the same sandbox (filesystem state survived).
    let after = rejoined
        .read_file(RunnerFileReadRequest {
            path: "hello.txt".into(),
        })
        .await?;
    anyhow::ensure!(
        after.contents == b"hello blaxel",
        "rejoined sandbox lost filesystem state"
    );
    println!(
        "live rejoin read: {}",
        String::from_utf8_lossy(&after.contents)
    );
    rejoined.close().await?;
    println!("live smoke OK: exec, file rw, preview, pause/resume/detach/rejoin, delete");
    Ok(())
}
