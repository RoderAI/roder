use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context, bail};
use sha2::{Digest, Sha512};
use tokio::io::AsyncWriteExt;

use super::release::{MANIFEST_BASE, MAX_DOWNLOAD, Release};
use super::{InstallProgress, archive, managed_directory, release, runtime_version};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Downloads only after the caller has obtained explicit user consent.
/// Dropping this future cancels download/verification and never publishes a partial runtime.
pub async fn install_runtime(
    progress: impl Fn(InstallProgress) + Send + Sync + 'static,
) -> anyhow::Result<PathBuf> {
    let root = managed_directory()?;
    let client = reqwest::Client::builder()
        // The official manifest supplies the final GCS object URL. Reject redirects,
        // including HTTPS downgrades and redirects to an untrusted asset host.
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(Duration::from_secs(60))
        .timeout(Duration::from_secs(900))
        .build()?;
    progress(InstallProgress::Manifest);
    let manifest_url = format!("{MANIFEST_BASE}/{}.json", release::target()?);
    let response = client.get(manifest_url).send().await?;
    let manifest = read_bounded(response, 16384).await?;
    let release = Release::parse(
        &serde_json::from_slice(&manifest).context("Invalid Antigravity release manifest")?,
        std::env::consts::OS,
        std::env::consts::ARCH,
        cfg!(target_env = "musl"),
    )?;
    private_directory(&root)?;
    let _lock = installation_lock(&root)?;
    let mut stage = Stage::new(&root)?;
    let archive_path = stage.path.join("download");
    let response = client.get(&release.url).send().await?;
    download(response, &archive_path, &release.digest, &progress).await?;
    progress(InstallProgress::Extracting);
    // A cancelled caller cannot remove files while the blocking extractor uses them.
    // Ownership returns only after extraction; publication remains in this async task.
    stage = tokio::task::spawn_blocking(move || {
        archive::extract(&archive_path, &stage.path, cfg!(windows))?;
        std::fs::remove_file(archive_path)?;
        Ok::<_, anyhow::Error>(stage)
    })
    .await
    .context("Antigravity extraction task failed")??;
    progress(InstallProgress::Verifying);
    let actual = runtime_version(&stage.path.join(release::executable_name())).await?;
    if actual != release.version {
        bail!(
            "Downloaded Antigravity version {actual} does not match manifest {}",
            release.version
        );
    }
    let path = publish(&root, stage, &release.version)?;
    progress(InstallProgress::Installed {
        path: path.clone(),
        version: actual,
    });
    Ok(path)
}

fn check_response(response: &reqwest::Response, limit: u64) -> anyhow::Result<()> {
    if !response.status().is_success() {
        bail!(
            "Official Antigravity download returned HTTP {}",
            response.status()
        );
    }
    if response.content_length().is_some_and(|size| size > limit) {
        bail!("Official Antigravity download exceeded its size limit");
    }
    Ok(())
}

async fn read_bounded(mut response: reqwest::Response, limit: u64) -> anyhow::Result<Vec<u8>> {
    check_response(&response, limit)?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len() as u64 + chunk.len() as u64 > limit {
            bail!("Antigravity release manifest exceeded its size limit");
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

async fn download(
    mut response: reqwest::Response,
    path: &Path,
    expected_digest: &str,
    progress: &(impl Fn(InstallProgress) + Send + Sync),
) -> anyhow::Result<()> {
    check_response(&response, MAX_DOWNLOAD)?;
    let total = response.content_length();
    // Create synchronously: cancellation must not leave a queued open recreating
    // a file while the staging directory's Drop cleanup removes its contents.
    let file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)?;
    let mut file = tokio::fs::File::from_std(file);
    let mut digest = Sha512::new();
    let mut bytes = 0u64;
    progress(InstallProgress::Downloading { bytes, total });
    while let Some(chunk) = response.chunk().await? {
        bytes += chunk.len() as u64;
        if bytes > MAX_DOWNLOAD {
            bail!("Antigravity runtime download exceeded its size limit");
        }
        digest.update(&chunk);
        file.write_all(&chunk).await?;
        progress(InstallProgress::Downloading { bytes, total });
    }
    if bytes == 0 || total.is_some_and(|total| total != bytes) {
        bail!("Antigravity download was empty or truncated");
    }
    file.sync_all().await?;
    let actual_digest = digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if actual_digest != expected_digest {
        bail!("Antigravity SHA-512 verification failed; no runtime was installed");
    }
    Ok(())
}

fn installation_lock(root: &Path) -> anyhow::Result<std::fs::File> {
    let path = root.join("install.lock");
    if std::fs::symlink_metadata(&path).is_ok_and(|meta| !meta.is_file()) {
        bail!("Antigravity installation lock is not a regular file");
    }
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(false).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let lock = options.open(path)?;
    lock.try_lock()
        .context("Another Antigravity installation is running; wait and retry")?;
    Ok(lock)
}

fn private_directory(path: &Path) -> anyhow::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => return Ok(()),
        Ok(_) => bail!("Antigravity installation path is not a real directory"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)?;
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(path)?;
    Ok(())
}

fn publish(root: &Path, mut stage: Stage, version: &str) -> anyhow::Result<PathBuf> {
    private_directory(&root.join("versions"))?;
    let id = format!("{version}-{}", stage.id);
    let destination = root.join("versions").join(&id);
    std::fs::rename(&stage.path, &destination)?;
    stage.path = destination;
    let path = std::fs::canonicalize(stage.path.join(release::executable_name()))?;
    let record = root.join(format!(".active-{}.json", stage.id));
    stage.record = Some(record.clone());
    {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&record)?;
        file.write_all(serde_json::to_string(&serde_json::json!({"release":id}))?.as_bytes())?;
        file.sync_all()?;
    }
    // Same-filesystem rename: readers see either the previous complete release or this one.
    std::fs::rename(record, root.join("active.json"))?;
    stage.record = None;
    stage.keep = true;
    Ok(path)
}

struct Stage {
    path: PathBuf,
    id: String,
    record: Option<PathBuf>,
    keep: bool,
}

impl Stage {
    fn new(root: &Path) -> anyhow::Result<Self> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let id = format!(
            "{now}-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        );
        let path = root.join(format!(".install-{id}"));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        // Never take ownership of an already-existing directory, even if its
        // unpredictable staging name was guessed by another process.
        builder.create(&path)?;
        Ok(Self {
            path,
            id,
            record: None,
            keep: false,
        })
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        if let Some(record) = &self.record {
            let _ = std::fs::remove_file(record);
        }
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(test)]
#[path = "install_tests.rs"]
mod tests;
