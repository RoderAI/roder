use super::*;
use ring::signature::{Ed25519KeyPair, KeyPair};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn channel(origin: &str) -> CaptureChannel {
    CaptureChannel::new(
        CaptureChannelPin {
            workspace: "snapshot-test".into(),
            source: "owned".into(),
            external_id: "owned-id".into(),
            origin: origin.into(),
            preview_name: "verify-capture-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            request_digest: "a".repeat(64),
        },
        deadline(),
    )
    .unwrap()
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(2)
}
fn binding(max_expires_at_ms: u64) -> SourceApiBootstrapBinding<'static> {
    SourceApiBootstrapBinding {
        source_id: "source_owned",
        checkpoint_id: "checkpoint_owned",
        admission_epoch: 3,
        max_expires_at_ms,
    }
}
fn keys() -> (Ed25519KeyPair, String) {
    let key = Ed25519KeyPair::from_seed_unchecked(&[11; 32]).unwrap();
    let mut public = PREFIX.to_vec();
    public.extend_from_slice(key.public_key().as_ref());
    (key, STANDARD.encode(public))
}
fn body() -> Value {
    let now = now_ms().unwrap();
    json!({"schema_version":1,"operation":"start-neutral-api","purpose":"source","source_id":"source_owned","target_id":"source_owned",
        "checkpoint_id":"checkpoint_owned","request_digest":"a".repeat(64),"admission_epoch":3,"issued_at_ms":now-10,
        "expires_at_ms":now+10000,"nonce":"b".repeat(32)})
}
fn ticket(value: &Value, key: &Ed25519KeyPair) -> Value {
    let bytes = serde_json::to_vec(value).unwrap();
    json!({"schema_version":1,"body":STANDARD.encode(&bytes),"signature":STANDARD.encode(key.sign(&bytes).as_ref())})
}
async fn read_request(socket: &mut tokio::net::TcpStream) -> String {
    let mut bytes = vec![];
    loop {
        let mut part = [0; 1024];
        let n = socket.read(&mut part).await.unwrap();
        assert!(n > 0 && bytes.len() + n <= 4096);
        bytes.extend_from_slice(&part[..n]);
        if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            let headers = std::str::from_utf8(&bytes[..end]).unwrap();
            let length = headers
                .lines()
                .filter_map(|line| line.split_once(':'))
                .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                .map(|(_, value)| value.trim().parse::<usize>().unwrap())
                .unwrap_or(0);
            if bytes.len() >= end + 4 + length {
                return String::from_utf8(bytes).unwrap();
            }
        }
    }
}
async fn server(
    code: u16,
    response: Option<String>,
    headers: &str,
) -> (CaptureChannel, tokio::task::JoinHandle<String>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let channel = channel(&format!("http://{}", listener.local_addr().unwrap()));
    let headers = headers.to_owned();
    let handle = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let request = read_request(&mut socket).await;
        if let Some(response) = response {
            let wire = format!(
                "HTTP/1.1 {code} Test\r\nContent-Type: application/json\r\nCache-Control: no-store\r\nConnection: close\r\nContent-Length: {}\r\n{headers}\r\n{response}",
                response.len()
            );
            let _ = socket.write_all(wire.as_bytes()).await;
        }
        drop(socket);
        assert!(
            tokio::time::timeout(Duration::from_millis(40), listener.accept())
                .await
                .is_err(),
            "bootstrap delivery retried"
        );
        request
    });
    (channel, handle)
}
fn receipt() -> Value {
    json!({"ok":true,"neutral_api_started":true,"full_vm_seal":false})
}

#[tokio::test]
async fn signed_source_bootstrap_sends_one_exact_credential_free_post() {
    let (channel, server) = server(200, Some(receipt().to_string()), "").await;
    let (key, public) = keys();
    let ticket = ticket(&body(), &key);
    let permit = channel
        .prepare_source_api_bootstrap(
            ticket.clone(),
            &public,
            &binding(now_ms().unwrap() + 12000),
            deadline(),
        )
        .unwrap();
    channel.bootstrap_source_api(permit).await.unwrap();
    let request = server.await.unwrap();
    assert!(request.starts_with("POST /bootstrap-api HTTP/1.1\r\n"));
    let headers = request
        .split("\r\n\r\n")
        .next()
        .unwrap()
        .to_ascii_lowercase();
    assert!(
        headers.contains("content-type: application/json")
            && headers.contains("accept-encoding: identity")
            && headers.contains("user-agent: node")
    );
    for forbidden in [
        "authorization:",
        "cookie:",
        "x-blaxel-workspace:",
        "x-vex-lease-token:",
    ] {
        assert!(!headers.contains(forbidden));
    }
    let value: Value = serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(value, json!({"operation":"bootstrap-api","ticket":ticket}));
}

#[test]
fn signed_source_bootstrap_refuses_foreign_expired_unbounded_or_unsigned_data_before_transport() {
    let channel = channel("http://127.0.0.1:1");
    let (key, public) = keys();
    for mode in 0..20 {
        let mut value = body();
        let mut expected = binding(now_ms().unwrap() + 12000);
        match mode {
            0 => value["source_id"] = json!("foreign_source"),
            1 => value["target_id"] = json!("foreign_target"),
            2 => value["checkpoint_id"] = json!("foreign_checkpoint"),
            3 => value["admission_epoch"] = json!(4),
            4 => value["request_digest"] = json!("c".repeat(64)),
            5 => value["purpose"] = json!("child"),
            6 => value["operation"] = json!("activate-browser"),
            7 => value["schema_version"] = json!(2),
            8 => value["expires_at_ms"] = json!(now_ms().unwrap() - 1),
            9 => value["issued_at_ms"] = json!(now_ms().unwrap() + 1000),
            10 => value["expires_at_ms"] = json!(now_ms().unwrap() + 16000),
            11 => expected.max_expires_at_ms = now_ms().unwrap() + 500,
            12 => value["nonce"] = json!("not-a-nonce"),
            13 => value["secret"] = json!("private-canary"),
            14 => expected.source_id = "bad/source",
            15 => expected.admission_epoch = i32::MAX as u32 + 1,
            _ => (),
        }
        let mut wire = ticket(&value, &key);
        match mode {
            16 => wire["extra"] = json!("private-canary"),
            17 => wire["signature"] = json!(STANDARD.encode([0; 64])),
            18 => {
                let bytes = format!("{} ", value);
                wire["body"] = json!(STANDARD.encode(bytes.as_bytes()));
                wire["signature"] = json!(STANDARD.encode(key.sign(bytes.as_bytes()).as_ref()));
            }
            19 => wire["body"] = json!("private-canary"),
            _ => (),
        }
        let error = channel
            .prepare_source_api_bootstrap(wire, &public, &expected, deadline())
            .err()
            .unwrap();
        assert_eq!(error.outcome, crate::forks::Outcome::Rejected);
        assert!(!format!("{error:?}").contains("private-canary"));
    }
}

#[tokio::test]
async fn signed_source_bootstrap_unknown_receipts_and_disconnects_never_retry() {
    for mode in 0..11 {
        let mut value = receipt();
        match mode {
            1 => value["full_vm_seal"] = json!(true),
            2 => value["secret"] = json!("private-canary"),
            3 => value["neutral_api_started"] = json!(false),
            _ => (),
        }
        let response = match mode {
            0 => None,
            4 => Some("private-canary".repeat(100)),
            5 => Some(format!("{value}\n")),
            _ => Some(value.to_string()),
        };
        let code = if mode == 6 {
            307
        } else if mode == 7 {
            503
        } else {
            200
        };
        let headers = match mode {
            6 => "Location: http://127.0.0.1:1/private-canary\r\n",
            8 => "Content-Type: text/html\r\n",
            9 => "Cache-Control: public\r\n",
            10 => "Content-Encoding: gzip\r\n",
            _ => "",
        };
        let (channel, server) = server(code, response, headers).await;
        let (key, public) = keys();
        let permit = channel
            .prepare_source_api_bootstrap(
                ticket(&body(), &key),
                &public,
                &binding(now_ms().unwrap() + 12000),
                deadline(),
            )
            .unwrap();
        let error = channel.bootstrap_source_api(permit).await.unwrap_err();
        assert_eq!(error.outcome, crate::forks::Outcome::Unknown);
        assert!(!format!("{error:?}").contains("private-canary"));
        assert!(server.await.unwrap().starts_with("POST /bootstrap-api "));
    }
}

#[tokio::test]
async fn signed_source_bootstrap_permit_cannot_change_route_or_outlive_its_deadline() {
    let first = channel("http://127.0.0.1:1");
    let second = channel("http://127.0.0.1:2");
    let (key, public) = keys();
    let permit = first
        .prepare_source_api_bootstrap(
            ticket(&body(), &key),
            &public,
            &binding(now_ms().unwrap() + 12000),
            deadline(),
        )
        .unwrap();
    assert_eq!(
        second
            .bootstrap_source_api(permit)
            .await
            .unwrap_err()
            .outcome,
        crate::forks::Outcome::Rejected
    );
    let permit = first
        .prepare_source_api_bootstrap(
            ticket(&body(), &key),
            &public,
            &binding(now_ms().unwrap() + 12000),
            Instant::now() + Duration::from_millis(10),
        )
        .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        first
            .bootstrap_source_api(permit)
            .await
            .unwrap_err()
            .outcome,
        crate::forks::Outcome::Rejected
    );
}

#[tokio::test]
async fn signed_source_bootstrap_cancelled_post_recovers_by_status_without_replay() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let channel = channel(&format!("http://{}", listener.local_addr().unwrap()));
    let received = std::sync::Arc::new(tokio::sync::Notify::new());
    let signal = received.clone();
    let server = tokio::spawn(async move {
        let (mut first, _) = listener.accept().await.unwrap();
        let post = read_request(&mut first).await;
        signal.notify_one();
        let (mut second, _) = listener.accept().await.unwrap();
        let get = read_request(&mut second).await;
        let body=json!({"ok":true,"schema_version":1,"request_digest":"a".repeat(64),"state":"complete","receipt":receipt(),"full_vm_seal":false}).to_string();
        second.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
        drop(first);
        drop(second);
        assert!(
            tokio::time::timeout(Duration::from_millis(40), listener.accept())
                .await
                .is_err()
        );
        (post, get)
    });
    let (key, public) = keys();
    let permit = channel
        .prepare_source_api_bootstrap(
            ticket(&body(), &key),
            &public,
            &binding(now_ms().unwrap() + 12000),
            deadline(),
        )
        .unwrap();
    {
        let work = channel.bootstrap_source_api(permit);
        tokio::pin!(work);
        tokio::select! {_=received.notified()=>(),result=&mut work=>panic!("held startup finished: {}",result.is_ok())}
    }
    assert_eq!(
        channel.bootstrap_status(deadline()).await.unwrap(),
        super::super::CaptureChannelState::Complete
    );
    let (post, get) = server.await.unwrap();
    assert!(post.starts_with("POST /bootstrap-api "));
    assert!(get.starts_with(&format!("GET /bootstrap-status/{} ", "a".repeat(64))));
}
