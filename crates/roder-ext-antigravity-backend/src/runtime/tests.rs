use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::json;
use sha2::{Digest, Sha512};

use super::{archive, managed_runtime, parse_version, release};

static NEXT: AtomicU64 = AtomicU64::new(1);

pub(super) struct Temp(pub PathBuf);

impl Temp {
    pub fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "roder-install-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub(super) fn digest(bytes: &[u8]) -> String {
    Sha512::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(super) fn tarball(path: &Path, entries: &[(&str, &[u8], tar::EntryType)]) {
    let file = std::fs::File::create(path).unwrap();
    let gzip = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    let mut archive = tar::Builder::new(gzip);
    for (name, bytes, kind) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o700);
        header.set_entry_type(*kind);
        if kind.is_symlink() || kind.is_hard_link() {
            header.set_link_name("/etc/passwd").unwrap();
        }
        header.set_cksum();
        archive.append_data(&mut header, name, *bytes).unwrap();
    }
    archive.into_inner().unwrap().finish().unwrap();
}

#[test]
fn target_and_manifest_validation_match_official_assets() {
    for (os, arch, musl, target, folder, asset) in [
        (
            "linux",
            "x86_64",
            false,
            "linux_amd64",
            "linux-x64",
            "cli_linux_x64.tar.gz",
        ),
        (
            "linux",
            "aarch64",
            true,
            "linux_arm64_musl",
            "linux-arm-musl",
            "cli_linux_arm64_musl.tar.gz",
        ),
        (
            "macos",
            "aarch64",
            false,
            "darwin_arm64",
            "darwin-arm",
            "cli_mac_arm64.tar.gz",
        ),
        (
            "windows",
            "x86_64",
            false,
            "windows_amd64",
            "windows-x64",
            "cli_windows_x64.exe",
        ),
    ] {
        assert_eq!(release::target_for(os, arch, musl).unwrap(), target);
        let value = json!({
            "version":"1.5.18",
            "url":format!("https://storage.googleapis.com/antigravity-public/antigravity-cli/1.5.18-123/{folder}/{asset}"),
            "sha512":"A".repeat(128)
        });
        assert_eq!(
            release::Release::parse(&value, os, arch, musl)
                .unwrap()
                .digest,
            "a".repeat(128)
        );
        for url in [
            value["url"].as_str().unwrap().replace("https:", "http:"),
            value["url"]
                .as_str()
                .unwrap()
                .replace("storage.googleapis.com", "evil.example"),
            format!("{}?override=1", value["url"].as_str().unwrap()),
            value["url"].as_str().unwrap().replace("-123/", "-abc/"),
            value["url"]
                .as_str()
                .unwrap()
                .replace(&format!("/{folder}/"), "/../"),
        ] {
            let mut invalid = value.clone();
            invalid["url"] = url.into();
            assert!(release::Release::parse(&invalid, os, arch, musl).is_err());
        }
        for version in [
            "1.5.0",
            "v1.5.18",
            "1.5.18-beta.1",
            "1.5.18+build",
            "01.5.18",
        ] {
            let mut invalid = value.clone();
            invalid["version"] = version.into();
            assert!(release::Release::parse(&invalid, os, arch, musl).is_err());
        }
    }
    assert!(release::target_for("freebsd", "x86_64", false).is_err());
    assert!(release::target_for("linux", "x86", false).is_err());
}

#[test]
fn version_validation_rejects_old_or_unrecognizable_runtime() {
    assert_eq!(parse_version("1.5.18\n").unwrap(), "1.5.18");
    assert_eq!(parse_version("Antigravity CLI v1.5.1\n").unwrap(), "1.5.1");
    for invalid in ["", "Editor 2", "1.5.0", "1.5.1-beta.1"] {
        assert!(parse_version(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn extraction_requires_one_regular_expected_file() {
    let temp = Temp::new();
    let archive_path = temp.0.join("archive.tar.gz");
    for entries in [
        vec![("wrong", b"payload".as_slice(), tar::EntryType::Regular)],
        vec![(
            release::executable_name(),
            b"".as_slice(),
            tar::EntryType::Regular,
        )],
        vec![(
            release::executable_name(),
            b"".as_slice(),
            tar::EntryType::Symlink,
        )],
        vec![(
            release::executable_name(),
            b"".as_slice(),
            tar::EntryType::Link,
        )],
        vec![(
            release::executable_name(),
            b"".as_slice(),
            tar::EntryType::Directory,
        )],
        vec![
            (
                release::executable_name(),
                b"payload".as_slice(),
                tar::EntryType::Regular,
            ),
            ("extra", b"payload".as_slice(), tar::EntryType::Regular),
        ],
    ] {
        let output = Temp::new();
        tarball(&archive_path, &entries);
        assert!(archive::extract(&archive_path, &output.0, false).is_err());
    }
    let output = Temp::new();
    tarball(
        &archive_path,
        &[(
            release::executable_name(),
            b"payload",
            tar::EntryType::Regular,
        )],
    );
    archive::extract(&archive_path, &output.0, false).unwrap();
    assert_eq!(
        std::fs::read(output.0.join(release::executable_name())).unwrap(),
        b"payload"
    );
}

#[test]
fn extraction_rejects_raw_traversal_oversized_and_corrupt_archives() {
    use std::io::Write;
    let temp = Temp::new();
    for (name, size) in [
        ("../escaped", 4),
        ("/absolute", 4),
        (release::executable_name(), release::MAX_EXECUTABLE + 1),
    ] {
        let path = temp.0.join("malicious.tar.gz");
        let gzip = flate2::write::GzEncoder::new(
            std::fs::File::create(&path).unwrap(),
            flate2::Compression::default(),
        );
        let mut builder = tar::Builder::new(gzip);
        let mut header = tar::Header::new_gnu();
        header.set_mode(0o700);
        header.set_size(size);
        header.set_entry_type(tar::EntryType::Regular);
        // Construct an adversarial raw header without tar::Builder's path checks.
        header.as_mut_bytes()[..name.len()].copy_from_slice(name.as_bytes());
        header.set_cksum();
        builder.append(&header, b"data".as_slice()).unwrap();
        builder.into_inner().unwrap().finish().unwrap();
        let output = Temp::new();
        assert!(archive::extract(&path, &output.0, false).is_err());
        assert!(!temp.0.join("escaped").exists());
    }
    let path = temp.0.join("corrupt.tar.gz");
    tarball(
        &path,
        &[(
            release::executable_name(),
            b"payload",
            tar::EntryType::Regular,
        )],
    );
    let mut bytes = std::fs::read(&path).unwrap();
    let last = bytes.len() - 8;
    bytes[last] ^= 0xff;
    std::fs::File::create(&path)
        .unwrap()
        .write_all(&bytes)
        .unwrap();
    let output = Temp::new();
    assert!(archive::extract(&path, &output.0, false).is_err());
}

#[test]
fn managed_record_rejects_traversal_and_links() {
    let temp = Temp::new();
    assert!(managed_runtime(&temp.0).unwrap().is_none());
    for release in ["../outside", "/absolute", ".", "..", "ok/../bad"] {
        std::fs::write(
            temp.0.join("active.json"),
            json!({"release":release}).to_string(),
        )
        .unwrap();
        assert!(managed_runtime(&temp.0).is_err());
    }
    std::fs::remove_file(temp.0.join("active.json")).unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("/etc/passwd", temp.0.join("active.json")).unwrap();
        assert!(managed_runtime(&temp.0).is_err());
    }
}

#[cfg(unix)]
#[tokio::test]
async fn version_probe_checks_exit_status_and_version() {
    use std::os::unix::fs::PermissionsExt;
    let temp = Temp::new();
    let binary = temp.0.join("agy");
    for (script, expected) in [
        ("#!/bin/sh\necho 1.5.18\n", true),
        ("#!/bin/sh\necho 1.5.18\nexit 1\n", false),
        ("#!/bin/sh\necho 1.4.0\n", false),
        ("#!/bin/sh\nhead -c 70000 /dev/zero\n", false),
    ] {
        std::fs::write(&binary, script).unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(super::runtime_version(&binary).await.is_ok(), expected);
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn cancelled_version_probe_kills_workers_and_removes_private_scratch() {
    use std::os::unix::fs::PermissionsExt;
    let temp = Temp::new();
    let binary = temp.0.join("agy");
    let scratch_record = temp.0.join("scratch");
    let worker_record = temp.0.join("worker");
    std::fs::write(&binary, format!(
        "#!/bin/sh\nprintf '%s' \"$TMPDIR\" > '{}'\nsleep 30 &\nprintf '%s' \"$!\" > '{}'\nwait\n",
        scratch_record.display(), worker_record.display()
    )).unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let task = tokio::spawn(async move { super::runtime_version(&binary).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !worker_record.exists() || std::fs::metadata(&worker_record).unwrap().len() == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let scratch = PathBuf::from(std::fs::read_to_string(scratch_record).unwrap());
    let pid = std::fs::read_to_string(worker_record).unwrap();
    assert!(scratch.exists());
    task.abort();
    let _ = task.await;
    assert!(!scratch.exists());
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let running = std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .is_ok_and(|stat| !stat.split(") ").nth(1).unwrap_or("").starts_with('Z'));
            if !running {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}
