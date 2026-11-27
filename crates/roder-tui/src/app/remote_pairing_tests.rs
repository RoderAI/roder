use super::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

struct PairingFile {
    path: PathBuf,
}

impl PairingFile {
    fn new() -> Self {
        static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let unique = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "roder-remote-pairing-{}-{timestamp}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir(&dir).unwrap();
        Self {
            path: dir.join("remote-pairing.json"),
        }
    }

    fn bytes(&self) -> Vec<u8> {
        std::fs::read(&self.path).unwrap()
    }
}

impl Drop for PairingFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.path.parent().unwrap());
    }
}

fn controller(listen: &str) -> RemotePanelController {
    RemotePanelController::with_listen(
        Arc::new(AppServer::new(Arc::new(
            roder_core::Runtime::fake().expect("fake runtime"),
        ))),
        listen.to_string(),
        None,
    )
}

async fn start_at(controller: &mut RemotePanelController, path: &Path) -> anyhow::Result<()> {
    controller.pairing_path = Some(path.to_path_buf());
    controller.start().await
}

#[tokio::test]
async fn secondary_listener_preserves_canonical_pairing_across_primary_restart() {
    let file = PairingFile::new();
    let mut primary = controller("ws://127.0.0.1:0");
    start_at(&mut primary, &file.path).await.unwrap();
    let canonical = pairing::load_or_create_at(&file.path).unwrap();
    assert_ne!(canonical.port, 0);
    let before = file.bytes();
    let auth_header = primary.copy_auth_header();

    let mut secondary = controller("ws://127.0.0.1:0");
    start_at(&mut secondary, &file.path).await.unwrap();
    assert!(secondary.is_running());
    assert_ne!(secondary.copy_url(), primary.copy_url());
    assert_eq!(secondary.copy_auth_header(), auth_header);
    assert_eq!(file.bytes(), before, "secondary must not rewrite pairing");

    // Even if the primary exits first, its browser pairing must remain valid
    // when it restarts while the secondary instance is still running.
    primary.stop().await.unwrap();
    let mut restarted = controller("ws://127.0.0.1:0");
    start_at(&mut restarted, &file.path).await.unwrap();
    assert_eq!(
        restarted
            .server
            .as_ref()
            .unwrap()
            .handle()
            .listen_addr
            .port(),
        canonical.port
    );
    assert_eq!(restarted.copy_auth_header(), auth_header);
    assert_eq!(file.bytes(), before);

    restarted.stop().await.unwrap();
    secondary.stop().await.unwrap();
}

#[tokio::test]
async fn startup_failure_does_not_rewrite_pairing_or_leave_a_listener() {
    let file = PairingFile::new();
    pairing::load_or_create_at(&file.path).unwrap();
    let before = file.bytes();
    let mut invalid = controller("not-a-websocket-listen-url");
    assert!(start_at(&mut invalid, &file.path).await.is_err());
    assert!(!invalid.is_running());
    assert!(invalid.copy_auth_header().is_none());
    assert_eq!(file.bytes(), before);
}

#[tokio::test]
async fn explicitly_occupied_port_reports_bind_failure_without_replacing_pairing() {
    let file = PairingFile::new();
    pairing::load_or_create_at(&file.path).unwrap();
    let before = file.bytes();
    let occupied = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut pinned = controller(&format!("ws://{}", occupied.local_addr().unwrap()));
    let error = start_at(&mut pinned, &file.path).await.unwrap_err();
    assert!(pairing_port_is_occupied(&error));
    assert!(!pinned.is_running());
    assert_eq!(file.bytes(), before);
}

#[tokio::test]
async fn explicit_rotation_uses_the_isolated_pairing_store() {
    let file = PairingFile::new();
    let mut listener = controller("ws://127.0.0.1:0");
    start_at(&mut listener, &file.path).await.unwrap();
    let original = pairing::load_or_create_at(&file.path).unwrap();
    listener.rotate_pairing().await.unwrap();
    let rotated = pairing::load_or_create_at(&file.path).unwrap();
    assert_ne!(rotated.token, original.token);
    assert_eq!(
        listener
            .server
            .as_ref()
            .unwrap()
            .handle()
            .listen_addr
            .port(),
        rotated.port
    );
    assert_eq!(
        listener.copy_auth_header(),
        Some(format!("Authorization: Bearer {}", rotated.token))
    );
    listener.stop().await.unwrap();
}

#[test]
fn only_address_in_use_allows_pairing_fallback() {
    let occupied = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::AddrInUse))
        .context("binding persisted pairing port");
    assert!(pairing_port_is_occupied(&occupied));
    for kind in [
        std::io::ErrorKind::PermissionDenied,
        std::io::ErrorKind::AddrNotAvailable,
        std::io::ErrorKind::InvalidInput,
    ] {
        let error = anyhow::Error::new(std::io::Error::from(kind))
            .context("binding persisted pairing port");
        assert!(!pairing_port_is_occupied(&error), "{kind:?}");
    }
    assert!(!pairing_port_is_occupied(&anyhow::anyhow!(
        "invalid config"
    )));
}
