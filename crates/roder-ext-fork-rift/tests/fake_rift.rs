//! Offline Rift provider tests against a fake `rift` script implementing
//! the adapter command contract (init/create/list/remove/gc). Verifies
//! command construction, path parsing, cleanup, and failure mapping with
//! no real Rift installed. An opt-in live test runs only with
//! `RODER_RIFT_LIVE=1` and `RIFT_BIN`.

use std::path::{Path, PathBuf};

use roder_api::forks::{
    ForkPolicy, ForkProvider, ForkReason, ForkRequest, ForkStatus, RemoveForkPolicy,
};
use roder_ext_fork_rift::{RiftConfig, RiftForkProvider};

const FAKE_RIFT: &str = r#"#!/bin/sh
# Fake rift implementing the Roder adapter contract. State lives next to
# the script; every invocation appends its argv to calls.log.
STATE_DIR="$(dirname "$0")"
echo "$@" >> "$STATE_DIR/calls.log"
case "$1" in
  init)
    exit 0 ;;
  create)
    NAME="$2"
    DEST="$4"
    if [ "$NAME" = "boom" ]; then
      echo "rift: simulated create failure" >&2
      exit 1
    fi
    mkdir -p "$DEST/$NAME"
    echo "creating snapshot $NAME"
    echo "$DEST/$NAME"
    printf '%s\t%s\n' "$NAME" "$DEST/$NAME" >> "$STATE_DIR/forks.tsv"
    exit 0 ;;
  list)
    [ -f "$STATE_DIR/forks.tsv" ] && cat "$STATE_DIR/forks.tsv"
    exit 0 ;;
  remove)
    TARGET="$2"
    rm -rf "$TARGET"
    if [ -f "$STATE_DIR/forks.tsv" ]; then
      grep -v "	$TARGET$" "$STATE_DIR/forks.tsv" > "$STATE_DIR/forks.tmp" || true
      mv "$STATE_DIR/forks.tmp" "$STATE_DIR/forks.tsv"
    fi
    exit 0 ;;
  gc)
    exit 0 ;;
  *)
    echo "unknown command $1" >&2
    exit 2 ;;
esac
"#;

struct Harness {
    provider: RiftForkProvider,
    source: PathBuf,
    state_dir: PathBuf,
}

fn harness(label: &str) -> Harness {
    let base = std::env::temp_dir().join(format!("roder-rift-{label}-{}", uuid::Uuid::new_v4()));
    let source = base.join("source");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("README.md"), "# src\n").unwrap();
    let state_dir = base.join("bin");
    std::fs::create_dir_all(&state_dir).unwrap();
    let script = state_dir.join("rift");
    std::fs::write(&script, FAKE_RIFT).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    Harness {
        provider: RiftForkProvider::new(RiftConfig {
            rift_bin: script,
            base_dir: Some(base.join("forks")),
        }),
        source,
        state_dir,
    }
}

fn calls(harness: &Harness) -> Vec<String> {
    std::fs::read_to_string(harness.state_dir.join("calls.log"))
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

fn request(source: &Path, name: &str) -> ForkRequest {
    ForkRequest {
        source_workspace: source.to_path_buf(),
        name: Some(name.to_string()),
        reason: ForkReason::TaskLane,
        policy: ForkPolicy::default(),
        provider_config: serde_json::json!({}),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn create_list_resume_remove_through_the_fake_binary() {
    let harness = harness("lifecycle");

    let fork = harness
        .provider
        .create_fork(request(&harness.source, "lane-a"))
        .await
        .unwrap();
    assert_eq!(fork.provider_id, "rift");
    assert_eq!(fork.status, ForkStatus::Active);
    assert!(fork.workspace.is_dir());
    assert_eq!(fork.provenance.snapshot_id.as_deref(), Some("lane-a"));
    assert_eq!(fork.metadata["copyOnWrite"], true);

    // Command construction: init then create with --dest.
    let recorded = calls(&harness);
    assert_eq!(recorded[0], "init");
    assert!(recorded[1].starts_with("create lane-a --dest "), "{recorded:?}");

    let listed = harness.provider.list_forks(&harness.source).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].workspace, fork.workspace);
    assert_eq!(listed[0].status, ForkStatus::Active);

    let resumed = harness.provider.resume_fork(&fork.id).await.unwrap();
    assert_eq!(resumed.status, ForkStatus::Active);

    // Removal is path-confirmed.
    let denied = harness
        .provider
        .remove_fork(
            &fork.id,
            RemoveForkPolicy {
                confirm_workspace: PathBuf::from("/wrong"),
            },
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(denied.contains("confirmation_mismatch"), "{denied}");

    let removed = harness
        .provider
        .remove_fork(
            &fork.id,
            RemoveForkPolicy {
                confirm_workspace: fork.workspace.clone(),
            },
        )
        .await
        .unwrap();
    assert!(removed.removed);
    assert!(!fork.workspace.exists());
    assert!(harness.provider.list_forks(&harness.source).await.unwrap().is_empty());

    // Resuming the removed fork reports Missing.
    let resumed = harness.provider.resume_fork(&fork.id).await.unwrap();
    assert_eq!(resumed.status, ForkStatus::Missing);
}

#[tokio::test(flavor = "multi_thread")]
async fn failures_map_to_typed_errors() {
    let harness = harness("failures");

    // Simulated create failure surfaces code + bounded stderr.
    let error = harness
        .provider
        .create_fork(request(&harness.source, "boom"))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("command_failed"), "{error}");
    assert!(error.contains("simulated create failure"), "{error}");

    // Missing binary maps to binary_missing with guidance.
    let missing = RiftForkProvider::new(RiftConfig {
        rift_bin: PathBuf::from("/definitely/not/rift"),
        base_dir: None,
    });
    let error = missing
        .create_fork(request(&harness.source, "x"))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("binary_missing"), "{error}");
    assert!(error.contains("RODER_RIFT_BIN"), "{error}");

    // Unsafe fork names never reach the binary.
    let error = harness
        .provider
        .create_fork(request(&harness.source, "../escape"))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("alphanumeric"), "{error}");
    assert!(!calls(&harness).iter().any(|line| line.contains("escape")));
}

/// Opt-in live check against a real Rift binary:
///
/// ```sh
/// RODER_RIFT_LIVE=1 RIFT_BIN="$(command -v rift)" \
///   cargo test -p roder-ext-fork-rift -- --ignored --nocapture
/// ```
///
/// Upstream Rift is pre-1.0; if its CLI flags differ from the adapter
/// contract this test documents the adaptation point.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "live rift check; set RODER_RIFT_LIVE=1 and RIFT_BIN"]
async fn live_rift_round_trip() {
    if std::env::var("RODER_RIFT_LIVE").ok().as_deref() != Some("1") {
        eprintln!("set RODER_RIFT_LIVE=1 and RIFT_BIN to run the live rift check");
        return;
    }
    let rift_bin = PathBuf::from(std::env::var("RIFT_BIN").expect("RIFT_BIN"));
    let harness_dir =
        std::env::temp_dir().join(format!("roder-rift-live-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(harness_dir.join("source")).unwrap();
    let provider = RiftForkProvider::new(RiftConfig {
        rift_bin,
        base_dir: Some(harness_dir.join("forks")),
    });
    let fork = provider
        .create_fork(request(&harness_dir.join("source"), "live-check"))
        .await
        .unwrap();
    eprintln!("live rift fork created at {}", fork.workspace.display());
    provider
        .remove_fork(
            &fork.id,
            RemoveForkPolicy {
                confirm_workspace: fork.workspace.clone(),
            },
        )
        .await
        .unwrap();
}
