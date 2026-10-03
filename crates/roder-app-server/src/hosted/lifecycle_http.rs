//! Bounded HTTP lifecycle control on the gateway's existing listener.
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

/// The embedding host authenticates the signature and validates allocation,
/// process, expiry, and command generation before changing admission.
#[async_trait::async_trait]
pub trait HostedLifecycleHandler: Send + Sync {
    async fn handle(&self, signature: Option<&str>, body: &[u8]) -> HostedLifecycleResponse;
}

pub struct HostedLifecycleResponse {
    pub status: u16,
    pub body: serde_json::Value,
}

const MAX_HEADER: usize = 4096;
const MAX_BODY: usize = 4096;
const SIGNATURE: &str = "x-roder-lifecycle-signature";

pub(crate) async fn try_handle(
    stream: &mut TcpStream,
    handler: Option<&Arc<dyn HostedLifecycleHandler>>,
) -> bool {
    // Peek only: ordinary WebSocket handshakes still use their existing auth
    // path. Wait for a whole request line so TCP fragmentation is harmless.
    let line = tokio::time::timeout(Duration::from_secs(2), async {
        let mut bytes = [0; 512];
        loop {
            let size = stream.peek(&mut bytes).await?;
            if size == 0 || size == bytes.len() || bytes[..size].contains(&b'\n') {
                return Ok::<_, std::io::Error>(bytes[..size].to_vec());
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await;
    let Ok(Ok(line)) = line else {
        return true;
    };
    if !line.starts_with(b"POST /lifecycle HTTP/1.1\r\n") {
        return false;
    }
    let response = match tokio::time::timeout(Duration::from_secs(5), read_request(stream)).await {
        Ok(Ok((signature, body))) => match handler {
            Some(handler) => handler.handle(signature.as_deref(), &body).await,
            None => error(404),
        },
        Ok(Err(())) => error(400),
        Err(_) => error(408),
    };
    let body = serde_json::to_vec(&response.body).unwrap_or_default();
    let status = if (200..=599).contains(&response.status) {
        response.status
    } else {
        500
    };
    let head = format!(
        "HTTP/1.1 {status} Lifecycle\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    let _ = tokio::time::timeout(Duration::from_secs(2), async {
        stream.write_all(head.as_bytes()).await?;
        stream.write_all(&body).await
    })
    .await;
    true
}

fn error(status: u16) -> HostedLifecycleResponse {
    HostedLifecycleResponse {
        status,
        body: serde_json::json!({"error":"lifecycle_request_rejected"}),
    }
}

async fn read_request(stream: &mut TcpStream) -> Result<(Option<String>, Vec<u8>), ()> {
    let mut bytes = Vec::new();
    let (head_size, body_size, signature) = loop {
        let mut chunk = [0; 1024];
        let size = stream.read(&mut chunk).await.map_err(|_| ())?;
        if size == 0 {
            return Err(());
        }
        bytes.extend_from_slice(&chunk[..size]);
        let mut headers = [httparse::EMPTY_HEADER; 32];
        let mut request = httparse::Request::new(&mut headers);
        match request.parse(&bytes).map_err(|_| ())? {
            httparse::Status::Partial if bytes.len() <= MAX_HEADER => continue,
            httparse::Status::Partial => return Err(()),
            httparse::Status::Complete(head_size) => {
                if head_size > MAX_HEADER
                    || request.method != Some("POST")
                    || request.path != Some("/lifecycle")
                {
                    return Err(());
                }
                let mut body_size = None;
                let mut signature = None;
                for header in request.headers {
                    if header.name.eq_ignore_ascii_case("transfer-encoding") {
                        return Err(());
                    }
                    if header.name.eq_ignore_ascii_case("content-length") {
                        if body_size.is_some() {
                            return Err(());
                        }
                        let value = std::str::from_utf8(header.value).map_err(|_| ())?;
                        if !value.bytes().all(|byte| byte.is_ascii_digit()) {
                            return Err(());
                        }
                        body_size = Some(value.parse::<usize>().map_err(|_| ())?);
                    }
                    if header.name.eq_ignore_ascii_case(SIGNATURE) {
                        if signature.is_some() {
                            return Err(());
                        }
                        signature = Some(
                            std::str::from_utf8(header.value)
                                .map_err(|_| ())?
                                .to_owned(),
                        );
                    }
                }
                let body_size = body_size.ok_or(())?;
                if body_size > MAX_BODY {
                    return Err(());
                }
                break (head_size, body_size, signature);
            }
        }
    };
    while bytes.len() < head_size + body_size {
        let remaining = head_size + body_size - bytes.len();
        let mut chunk = [0; 1024];
        let size = stream
            .read(&mut chunk[..remaining.min(1024)])
            .await
            .map_err(|_| ())?;
        if size == 0 {
            return Err(());
        }
        bytes.extend_from_slice(&chunk[..size]);
    }
    if bytes.len() != head_size + body_size {
        return Err(());
    }
    Ok((signature, bytes.split_off(head_size)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Handler(Arc<AtomicUsize>);
    #[async_trait::async_trait]
    impl HostedLifecycleHandler for Handler {
        async fn handle(&self, signature: Option<&str>, body: &[u8]) -> HostedLifecycleResponse {
            self.0.fetch_add(1, Ordering::SeqCst);
            assert_eq!(body, b"{\"action\":\"status\"}");
            HostedLifecycleResponse {
                status: if signature == Some("signed") {
                    200
                } else {
                    401
                },
                body: serde_json::json!({"ok":true}),
            }
        }
    }
    async fn request(raw: &[u8], fragmented: bool, enabled: bool) -> (String, usize) {
        let count = Arc::new(AtomicUsize::new(0));
        let handler: Arc<dyn HostedLifecycleHandler> = Arc::new(Handler(count.clone()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            assert!(try_handle(&mut stream, enabled.then_some(&handler)).await);
        });
        if fragmented {
            for part in raw.chunks(3) {
                client.write_all(part).await.unwrap();
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        } else {
            client.write_all(raw).await.unwrap();
        }
        let mut response = String::new();
        tokio::time::timeout(Duration::from_secs(3), client.read_to_string(&mut response))
            .await
            .unwrap()
            .unwrap();
        task.await.unwrap();
        (response, count.load(Ordering::SeqCst))
    }
    #[tokio::test]
    async fn fragmented_control_requests_preserve_signature_and_exact_body() {
        let raw = b"POST /lifecycle HTTP/1.1\r\nContent-Length: 19\r\nX-Roder-Lifecycle-Signature: signed\r\n\r\n{\"action\":\"status\"}";
        let (response, count) = request(raw, true, true).await;
        assert!(response.starts_with("HTTP/1.1 200"));
        assert_eq!(count, 1);
    }
    #[tokio::test]
    async fn malformed_or_oversized_framing_never_reaches_control_handler() {
        for headers in [
            "Content-Length: 4097",
            "Content-Length: 0\r\nContent-Length: 0",
            "Transfer-Encoding: chunked",
            "Content-Length: 0\r\nX-Roder-Lifecycle-Signature: a\r\nX-Roder-Lifecycle-Signature: b",
        ] {
            let raw = format!("POST /lifecycle HTTP/1.1\r\n{headers}\r\n\r\n");
            let (response, count) = request(raw.as_bytes(), false, true).await;
            assert!(response.starts_with("HTTP/1.1 400"), "{response}");
            assert_eq!(count, 0);
        }
    }
    #[tokio::test]
    async fn authentication_is_required_by_handler_and_absent_handler_is_closed() {
        let raw = b"POST /lifecycle HTTP/1.1\r\nContent-Length: 19\r\n\r\n{\"action\":\"status\"}";
        assert!(
            request(raw, false, true)
                .await
                .0
                .starts_with("HTTP/1.1 401")
        );
        let (response, count) = request(raw, false, false).await;
        assert!(response.starts_with("HTTP/1.1 404"));
        assert_eq!(count, 0);
    }
}
