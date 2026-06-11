//! Offline ADC token-acquisition tests (roadmap phase 69): authorized-user
//! refresh against a fake token endpoint with caching and expiry, the
//! gcloud CLI fallback via a fake binary, and the actionable
//! service-account rejection. No live Google credentials are touched.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use roder_ext_google_speech::adc::AdcTokenSource;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn temp_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("roder-adc-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Fake OAuth token endpoint returning sequential tokens.
async fn fake_token_endpoint(expires_in: u64) -> (String, Arc<AtomicUsize>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/token", listener.local_addr().unwrap());
    let hits = Arc::new(AtomicUsize::new(0));
    let task_hits = hits.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let mut buffer = [0u8; 8192];
            let n = stream.read(&mut buffer).await.unwrap_or(0);
            let request = String::from_utf8_lossy(&buffer[..n]).to_string();
            // The refresh grant must carry the stored refresh token.
            assert!(request.contains("grant_type=refresh_token"), "{request}");
            assert!(request.contains("refresh_token=rt-1"), "{request}");
            let count = task_hits.fetch_add(1, Ordering::SeqCst) + 1;
            let body = format!(
                r#"{{"access_token":"adc-token-{count}","expires_in":{expires_in},"token_type":"Bearer"}}"#
            );
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        }
    });
    (url, hits)
}

fn write_authorized_user_json(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("adc.json");
    std::fs::write(
        &path,
        r#"{"type":"authorized_user","client_id":"cid","client_secret":"cs","refresh_token":"rt-1"}"#,
    )
    .unwrap();
    path
}

#[tokio::test]
async fn authorized_user_adc_refreshes_and_caches_tokens() {
    let dir = temp_dir();
    let credentials = write_authorized_user_json(&dir);
    let (endpoint, hits) = fake_token_endpoint(3600).await;
    let source = AdcTokenSource::new(Some(credentials), &endpoint, "/nonexistent/gcloud");

    let first = source.access_token().await.unwrap();
    assert_eq!(first, "adc-token-1");
    // Cached: a second call within the expiry window makes no request.
    let second = source.access_token().await.unwrap();
    assert_eq!(second, "adc-token-1");
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn near_expiry_tokens_are_refreshed() {
    let dir = temp_dir();
    let credentials = write_authorized_user_json(&dir);
    // expires_in below the 60s slack: every call refreshes.
    let (endpoint, hits) = fake_token_endpoint(30).await;
    let source = AdcTokenSource::new(Some(credentials), &endpoint, "/nonexistent/gcloud");

    assert_eq!(source.access_token().await.unwrap(), "adc-token-1");
    assert_eq!(source.access_token().await.unwrap(), "adc-token-2");
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn service_account_keys_are_rejected_with_guidance() {
    let dir = temp_dir();
    let path = dir.join("sa.json");
    std::fs::write(
        &path,
        r#"{"type":"service_account","private_key":"-----BEGIN PRIVATE KEY-----"}"#,
    )
    .unwrap();
    let source = AdcTokenSource::new(Some(path), "http://127.0.0.1:1/token", "/nonexistent/gcloud");

    let error = source.access_token().await.unwrap_err().to_string();
    assert!(error.contains("RS256"), "{error}");
    assert!(error.contains("gcloud auth application-default login"), "{error}");
    assert!(!error.contains("BEGIN PRIVATE KEY"), "errors never echo key material");
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[tokio::test]
async fn gcloud_cli_fallback_uses_the_configured_binary() {
    use std::os::unix::fs::PermissionsExt;
    let dir = temp_dir();
    let bin = dir.join("fake-gcloud");
    std::fs::write(
        &bin,
        "#!/bin/sh\n[ \"$1 $2 $3\" = \"auth application-default print-access-token\" ] || exit 2\necho gcloud-token-1\n",
    )
    .unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();

    let source = AdcTokenSource::new(None, "http://127.0.0.1:1/token", bin.display().to_string());
    assert_eq!(source.access_token().await.unwrap(), "gcloud-token-1");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn unavailable_adc_fails_with_actionable_setup_guidance() {
    let source = AdcTokenSource::new(None, "http://127.0.0.1:1/token", "/nonexistent/gcloud");
    let error = source.access_token().await.unwrap_err().to_string();
    assert!(error.contains("gcloud auth application-default login"), "{error}");
}
