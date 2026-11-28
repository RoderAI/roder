//! Explicit, per-user installation of the official Antigravity CLI.
//! Discovery never installs, creates directories, or accesses the network.
mod archive;
mod install;
mod release;

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, bail};
use semver::Version;
use tokio::io::AsyncReadExt;

pub use install::install_runtime;

#[derive(Clone, Debug)]
pub enum InstallProgress {
    Manifest,
    Downloading { bytes: u64, total: Option<u64> },
    Extracting,
    Verifying,
    Installed { path: PathBuf, version: String },
}

/// Uses the same directory precedence as roder-config without a dependency cycle.
pub fn managed_directory() -> anyhow::Result<PathBuf> {
    let root = std::env::var_os("RODER_CONFIG_DIR")
        .or_else(|| std::env::var_os("RODER_DATA_DIR"))
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".roder")))
        .context("Cannot locate your Roder directory; set RODER_CONFIG_DIR")?;
    Ok(root
        .join("tools")
        .join("antigravity")
        .join(release::target()?))
}

/// A managed runtime takes priority so installing repairs an outdated PATH runtime.
pub fn find_runtime() -> anyhow::Result<Option<PathBuf>> {
    let managed = managed_directory()?;
    if let Some(path) = managed_runtime(&managed)? {
        return Ok(Some(path));
    }
    Ok(which::which("agy").ok())
}

fn managed_runtime(root: &Path) -> anyhow::Result<Option<PathBuf>> {
    match std::fs::symlink_metadata(root) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => bail!("Managed Antigravity root is not a real directory"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    let active = root.join("active.json");
    let metadata = match std::fs::symlink_metadata(&active) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_file() || metadata.len() > 4096 {
        bail!("Invalid managed Antigravity installation; reinstall through Roder setup");
    }
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(active)?)?;
    let id = value["release"]
        .as_str()
        .context("Invalid Antigravity release record")?;
    if id.is_empty()
        || id.starts_with('.')
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    {
        bail!("Invalid managed Antigravity release path");
    }
    let versions = root.join("versions");
    if !std::fs::symlink_metadata(&versions)?.is_dir() {
        bail!("Managed Antigravity versions path is not a real directory");
    }
    let directory = versions.join(id);
    if !std::fs::symlink_metadata(&directory)?.is_dir() {
        bail!("Managed Antigravity release directory is not a real directory");
    }
    let binary = directory.join(release::executable_name());
    let metadata = std::fs::symlink_metadata(&binary)?;
    if !metadata.is_file() || metadata.len() == 0 {
        bail!("Managed Antigravity executable is missing or invalid; reinstall through setup");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            bail!("Managed Antigravity runtime is not executable; reinstall through setup");
        }
    }
    Ok(Some(std::fs::canonicalize(binary)?))
}

/// Checks the official CLI version without authentication or a model request.
pub async fn runtime_version(path: &Path) -> anyhow::Result<String> {
    let scratch = crate::transport::Scratch::create()?;
    let mut command = tokio::process::Command::new(path);
    crate::clear_api_key_environment(&mut command);
    command
        .arg("--version")
        .env("AGY_CLI_DISABLE_AUTO_UPDATE", "true")
        .env("BROWSER", "false")
        .env("TMPDIR", scratch.path())
        .env("TMP", scratch.path())
        .env("TEMP", scratch.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let child = roder_api::spawn::spawn_retrying_busy(&mut command)
        .await
        .context("Cannot start Antigravity CLI version check")?;
    #[cfg(unix)]
    let group = child.id();
    let mut probe = VersionProbe {
        child,
        _scratch: scratch,
        #[cfg(unix)]
        group,
    };
    let mut stdout = probe
        .child
        .stdout
        .take()
        .context("Missing version output")?;
    let output = tokio::time::timeout(Duration::from_secs(90), async {
        let mut bytes = Vec::new();
        (&mut stdout).take(65537).read_to_end(&mut bytes).await?;
        if bytes.len() > 65536 {
            bail!("Antigravity version output exceeded its size limit");
        }
        let status = probe.child.wait().await?;
        if !status.success() {
            bail!("Antigravity --version exited unsuccessfully ({status})");
        }
        Ok::<_, anyhow::Error>(bytes)
    })
    .await
    .context("Antigravity version check timed out after 90 seconds")??;
    parse_version(&String::from_utf8_lossy(&output))
}

struct VersionProbe {
    child: tokio::process::Child,
    _scratch: crate::transport::Scratch,
    #[cfg(unix)]
    group: Option<u32>,
}

impl Drop for VersionProbe {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(group) = self.group {
            // All descendants of this version probe share its private process group.
            unsafe {
                libc::kill(-(group as i32), libc::SIGKILL);
            }
        }
        let _ = self.child.start_kill();
        let _ = self.child.try_wait();
    }
}

fn parse_version(output: &str) -> anyhow::Result<String> {
    let version = output
        .split_whitespace()
        .find_map(|word| Version::parse(word.trim_start_matches('v')).ok())
        .context("Antigravity CLI did not report a valid version")?;
    if version < Version::new(1, 5, 1) {
        bail!(
            "Antigravity {version} is too old; version 1.5.1 or newer is required. Install through Roder setup"
        );
    }
    Ok(version.to_string())
}

#[cfg(test)]
mod tests;
