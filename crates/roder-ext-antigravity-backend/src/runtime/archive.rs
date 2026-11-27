use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

use anyhow::{Context, bail};
use flate2::read::GzDecoder;

use super::release::{MAX_EXECUTABLE, executable_name};

pub(super) fn extract(archive: &Path, directory: &Path, windows: bool) -> anyhow::Result<()> {
    let binary = directory.join(executable_name());
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&binary)?;
    if windows {
        let mut input = File::open(archive)?.take(MAX_EXECUTABLE + 1);
        let size = std::io::copy(&mut input, &mut output)?;
        if size == 0 || size > MAX_EXECUTABLE {
            bail!("Antigravity executable exceeded its size limit or was empty");
        }
    } else {
        let decoder = GzDecoder::new(File::open(archive)?);
        let mut archive = tar::Archive::new(decoder);
        let mut entries = archive.entries()?;
        let mut entry = entries.next().context("Antigravity archive is empty")??;
        // Never unpack arbitrary paths, metadata, links or extra archive members.
        if entry.path_bytes().as_ref() != executable_name().as_bytes()
            || entry.header().entry_type() != tar::EntryType::Regular
        {
            bail!("Antigravity archive must contain only its regular executable");
        }
        let size = entry.size();
        if size == 0 || size > MAX_EXECUTABLE {
            bail!("Antigravity executable exceeded its size limit or was empty");
        }
        if std::io::copy(&mut entry, &mut output)? != size {
            bail!("Antigravity executable archive is truncated");
        }
        if entries.next().is_some() {
            bail!("Antigravity archive contains extra entries");
        }
        // Read to EOF to validate the gzip trailer/checksum as well as the tar payload.
        let mut decoder = archive.into_inner();
        let mut extra = [0u8; 8192];
        let mut padding = 0usize;
        loop {
            let count = decoder.read(&mut extra)?;
            if count == 0 {
                break;
            }
            padding += count;
            if padding > 1024 * 1024 || extra[..count].iter().any(|byte| *byte != 0) {
                bail!("Unexpected trailing data in Antigravity archive");
            }
        }
    }
    output.flush()?;
    output.sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
