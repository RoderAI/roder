use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn pin(origin: &str) -> CaptureChannelPin {
    CaptureChannelPin {
        workspace: "composal-experimental".into(),
        source: "owned-source".into(),
        external_id: "owned-external".into(),
        origin: origin.into(),
        request_digest: "a".repeat(64),
    }
}

fn status(state: &str, bootstrap: bool) -> Value {
    json!({"ok":true,"schema_version":1,"request_digest":"a".repeat(64),"state":state,
        "receipt":if state == "complete" { if bootstrap { json!({"ok":true,"neutral_api_started":true,"full_vm_seal":false}) }
            else { json!({"ok":true,"command_api_stopped":true,"parent_assets_removed":true,"full_vm_seal":false}) } } else { Value::Null },
        "full_vm_seal":false})
}

async fn server(
    code: u16,
    body: String,
    extra: &str,
) -> (CaptureChannel, tokio::task::JoinHandle<String>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let channel = CaptureChannel::new(
        pin(&format!("http://{}", listener.local_addr().unwrap())),
        deadline(),
    )
    .unwrap();
    let extra = extra.to_string();
    let handle = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        loop {
            let mut chunk = [0; 1024];
            let n = socket.read(&mut chunk).await.unwrap();
            assert!(n > 0 && bytes.len() < 8192);
            bytes.extend_from_slice(&chunk[..n]);
            if bytes.windows(4).any(|v| v == b"\r\n\r\n") {
                break;
            }
        }
        let response = format!(
            "HTTP/1.1 {code} Test\r\nContent-Type: application/json\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n{body}",
            body.len()
        );
        let _ = socket.write_all(response.as_bytes()).await;
        String::from_utf8(bytes).unwrap()
    });
    (channel, handle)
}

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(3)
}

#[test]
fn preview_urls_exclude_credentials_queries_nonregional_hosts_and_other_ports() {
    for value in [
        "https://abc.preview.bl.run",
        "https://abc.us-was-1.preview.bl.run/",
        "https://abc.eu-fra-1.preview.bl.run",
    ] {
        assert!(preview_origin(value).is_ok(), "{value}");
    }
    for value in [
        "http://abc.preview.bl.run",
        "https://user:private@abc.preview.bl.run",
        "https://abc.preview.bl.run?private",
        "https://abc.preview.bl.run/#private",
        "https://abc.preview.bl.run:4330",
        "https://abc.preview.bl.run/process",
        "https://abc.other.preview.bl.run",
        "https://abc.us-was-1.preview.bl.run.evil.test",
        "https://-abc.preview.bl.run",
        "https://abc-.preview.bl.run",
        "https://a.b.preview.bl.run",
        "https://localhost",
        "https://abc..preview.bl.run",
    ] {
        let error = preview_origin(value).unwrap_err();
        assert!(!format!("{error:?} {error}").contains("private"));
    }
}

#[tokio::test]
async fn native_status_uses_only_anonymous_fixed_requests_and_exact_receipts() {
    for bootstrap in [false, true] {
        let (channel, server) = server(200, status("complete", bootstrap).to_string(), "").await;
        assert_eq!(
            channel.status(bootstrap, deadline()).await.unwrap(),
            CaptureChannelState::Complete
        );
        let request = server.await.unwrap().to_ascii_lowercase();
        let operation = if bootstrap {
            "bootstrap-status"
        } else {
            "capture-status"
        };
        assert!(request.starts_with(&format!("get /{operation}/{} http/1.1\r\n", "a".repeat(64))));
        assert!(request.contains("accept-encoding: identity\r\n"));
        assert!(request.contains("user-agent: node\r\n"));
        for forbidden in [
            "authorization:",
            "cookie:",
            "x-blaxel",
            "x-vex",
            "composal-experimental",
            "owned-external",
        ] {
            assert!(!request.contains(forbidden));
        }
    }
}

#[tokio::test]
async fn status_refuses_changed_binding_fields_seal_claims_and_non_neutral_receipts() {
    for mode in 0..11 {
        let mut reply = status("complete", false);
        match mode {
            0 => reply["request_digest"] = json!("b".repeat(64)),
            1 => reply["schema_version"] = json!(2),
            2 => reply["full_vm_seal"] = json!(true),
            3 => reply["receipt"]["full_vm_seal"] = json!(true),
            4 => reply["receipt"]["secret"] = json!("private-canary"),
            5 => reply["secret"] = json!("private-canary"),
            6 => reply["receipt"]["command_api_stopped"] = json!(false),
            7 => reply["state"] = json!("pending"),
            8 => reply["ok"] = json!(false),
            9 => reply["receipt"] = Value::Null,
            _ => {
                reply.as_object_mut().unwrap().remove("receipt");
            }
        }
        let (channel, server) = server(200, reply.to_string(), "").await;
        let error = channel.capture_status(deadline()).await.unwrap_err();
        assert!(!format!("{error:?} {error}").contains("private-canary"));
        server.await.unwrap();
    }
}

#[tokio::test]
async fn noncomplete_status_is_read_only_and_retirement_is_capture_only() {
    for (value, expected) in [
        ("not_observed", CaptureChannelState::NotObserved),
        ("pending", CaptureChannelState::Pending),
        ("failed", CaptureChannelState::Failed),
        ("retired", CaptureChannelState::Retired),
    ] {
        let (channel, server) = server(200, status(value, false).to_string(), "").await;
        assert_eq!(channel.capture_status(deadline()).await.unwrap(), expected);
        server.await.unwrap();
    }
    let (channel, server) = server(200, status("retired", true).to_string(), "").await;
    assert!(channel.bootstrap_status(deadline()).await.is_err());
    server.await.unwrap();
}

#[tokio::test]
async fn redirects_oversized_duplicates_and_expired_reads_cannot_authorize_recovery() {
    for (code, body, headers) in [
        (
            302,
            status("complete", false).to_string(),
            "Location: http://127.0.0.1:1/private\r\n",
        ),
        (500, "private-canary".into(), ""),
        (200, "x".repeat(1025), ""),
        (
            200,
            status("complete", false).to_string().replacen(
                "\"ok\":true",
                "\"ok\":false,\"ok\":true",
                1,
            ),
            "",
        ),
    ] {
        let (channel, server) = server(code, body, headers).await;
        let error = channel.capture_status(deadline()).await.unwrap_err();
        assert!(!format!("{error:?} {error}").contains("private"));
        server.await.unwrap();
    }
    let channel = CaptureChannel::new(pin("http://127.0.0.1:1"), deadline()).unwrap();
    assert!(channel.capture_status(Instant::now()).await.is_err());
}

#[tokio::test]
async fn redirect_target_is_never_contacted_even_if_it_could_return_a_valid_receipt() {
    let target = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let location = format!(
        "Location: http://{}/private\r\n",
        target.local_addr().unwrap()
    );
    let (channel, server) = server(302, status("complete", false).to_string(), &location).await;
    assert!(channel.capture_status(deadline()).await.is_err());
    server.await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(50), target.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn a_stalled_body_expires_at_original_deadline_without_replaying_the_read() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let channel = CaptureChannel::new(
        pin(&format!("http://{}", listener.local_addr().unwrap())),
        deadline(),
    )
    .unwrap();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = [0; 2048];
        assert!(socket.read(&mut bytes).await.unwrap() > 0);
        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nCache-Control: no-store\r\nContent-Length: 100\r\n\r\n",
            )
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(250), listener.accept())
                .await
                .is_err()
        );
    });
    let began = Instant::now();
    assert!(
        tokio::time::timeout(
            Duration::from_secs(1),
            channel.capture_status(Instant::now() + Duration::from_millis(100))
        )
        .await
        .unwrap()
        .is_err()
    );
    assert!(began.elapsed() >= Duration::from_millis(80));
    task.await.unwrap();
}

#[tokio::test]
async fn encoded_responses_cannot_be_implicitly_decompressed_or_admitted() {
    let (channel, server) = server(
        200,
        status("complete", false).to_string(),
        "Content-Encoding: gzip\r\n",
    )
    .await;
    assert!(channel.capture_status(deadline()).await.is_err());
    server.await.unwrap();
}

#[tokio::test]
async fn later_callers_cannot_extend_the_channels_original_deadline() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let channel = CaptureChannel::new(
        pin(&format!("http://{}", listener.local_addr().unwrap())),
        Instant::now() + Duration::from_millis(10),
    )
    .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(channel.capture_status(deadline()).await.is_err());
    assert!(
        tokio::time::timeout(Duration::from_millis(20), listener.accept())
            .await
            .is_err()
    );
}
