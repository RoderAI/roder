use anyhow::{Context, bail};
use semver::Version;
use serde_json::Value;

pub(super) const MANIFEST_BASE: &str =
    "https://antigravity-cli-auto-updater-974169037036.us-central1.run.app/manifests";
pub(super) const MAX_DOWNLOAD: u64 = 512 * 1024 * 1024;
pub(super) const MAX_EXECUTABLE: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug)]
pub(super) struct Release {
    pub version: String,
    pub url: String,
    pub digest: String,
}

pub(super) fn target() -> anyhow::Result<String> {
    target_for(
        std::env::consts::OS,
        std::env::consts::ARCH,
        cfg!(target_env = "musl"),
    )
}

pub(super) fn target_for(os: &str, arch: &str, musl: bool) -> anyhow::Result<String> {
    let platform = match os {
        "macos" => "darwin",
        "linux" => "linux",
        "windows" => "windows",
        _ => bail!("Automatic Antigravity installation is not supported on {os}"),
    };
    let cpu = match arch {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        _ => bail!("Automatic Antigravity installation is not supported on {arch}"),
    };
    let suffix = if os == "linux" && musl { "_musl" } else { "" };
    Ok(format!("{platform}_{cpu}{suffix}"))
}

pub(super) fn executable_name() -> &'static str {
    if cfg!(windows) {
        "agy.exe"
    } else {
        "antigravity"
    }
}

impl Release {
    pub fn parse(value: &Value, os: &str, arch: &str, musl: bool) -> anyhow::Result<Self> {
        target_for(os, arch, musl)?;
        let version = value["version"]
            .as_str()
            .context("Release manifest missing version")?;
        let parsed = Version::parse(version).context("Invalid release version")?;
        if parsed < Version::new(1, 5, 1)
            || !parsed.pre.is_empty()
            || !parsed.build.is_empty()
            || parsed.to_string() != version
        {
            bail!("Antigravity release must be a stable version 1.5.1 or newer");
        }
        let url = value["url"]
            .as_str()
            .context("Release manifest missing URL")?;
        let digest = value["sha512"]
            .as_str()
            .context("Release manifest missing SHA-512")?;
        if digest.len() != 128 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            bail!("Invalid release SHA-512 digest");
        }
        let platform = match os {
            "macos" => "darwin",
            other => other,
        };
        let folder_cpu = if arch == "aarch64" { "arm" } else { "x64" };
        let asset_cpu = if arch == "aarch64" { "arm64" } else { "x64" };
        let suffix = if os == "linux" && musl { "_musl" } else { "" };
        let folder_suffix = if suffix.is_empty() { "" } else { "-musl" };
        let asset_os = if os == "macos" { "mac" } else { os };
        let extension = if os == "windows" { "exe" } else { "tar.gz" };
        let prefix =
            format!("https://storage.googleapis.com/antigravity-public/antigravity-cli/{version}-");
        let rest = url
            .strip_prefix(&prefix)
            .context("Untrusted Antigravity release URL")?;
        let (build, asset) = rest
            .split_once('/')
            .context("Invalid Antigravity release URL")?;
        let expected = format!(
            "{platform}-{folder_cpu}{folder_suffix}/cli_{asset_os}_{asset_cpu}{suffix}.{extension}"
        );
        if build.is_empty() || !build.bytes().all(|byte| byte.is_ascii_digit()) || asset != expected
        {
            bail!("Untrusted Antigravity release URL");
        }
        Ok(Self {
            version: version.into(),
            url: url.into(),
            digest: digest.to_ascii_lowercase(),
        })
    }
}
