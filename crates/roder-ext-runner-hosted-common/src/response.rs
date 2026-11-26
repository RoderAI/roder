use anyhow::{Context, bail};
use serde::de::DeserializeOwned;

pub(crate) async fn decode_response<T: DeserializeOwned>(
    response: reqwest::Response,
    operation: &str,
) -> anyhow::Result<T> {
    check_status(&response, operation)?;
    let body = response
        .bytes()
        .await
        .map_err(reqwest::Error::without_url)
        .with_context(|| format!("read {operation} response"))?;
    serde_json::from_slice(&body).map_err(|error| {
        anyhow::anyhow!(
            "decode {operation} response: invalid payload ({:?} at line {}, column {})",
            error.classify(),
            error.line(),
            error.column()
        )
    })
}

pub(crate) async fn decode_bytes(
    response: reqwest::Response,
    operation: &str,
) -> anyhow::Result<Vec<u8>> {
    check_status(&response, operation)?;
    let body = response
        .bytes()
        .await
        .map_err(reqwest::Error::without_url)
        .with_context(|| format!("read {operation} response"))?;
    Ok(body.to_vec())
}

pub(crate) async fn decode_empty(
    response: reqwest::Response,
    operation: &str,
) -> anyhow::Result<()> {
    check_status(&response, operation)?;
    response
        .bytes()
        .await
        .map_err(reqwest::Error::without_url)
        .with_context(|| format!("read {operation} response"))?;
    Ok(())
}

fn check_status(response: &reqwest::Response, operation: &str) -> anyhow::Result<()> {
    let status = response.status();
    if !status.is_success() {
        // Upstream error bodies are untrusted and can echo credentials.
        bail!("{operation} failed with {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    const SECRET: &str = "SYNTHETIC-c481d9-credential";

    async fn response(
        status: &str,
        body: &str,
        keep_body_open: bool,
    ) -> (reqwest::Response, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let content_length = body.len() + usize::from(keep_body_open);
        let wire = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {content_length}\r\nConnection: close\r\n\r\n{body}"
        );
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let _ = socket.read(&mut [0; 4096]).await.unwrap();
            socket.write_all(wire.as_bytes()).await.unwrap();
            if keep_body_open {
                std::future::pending::<()>().await;
            }
        });
        let response = reqwest::get(format!("http://{address}/")).await.unwrap();
        (response, server)
    }

    #[tokio::test]
    async fn failure_uses_status_without_waiting_for_or_displaying_upstream_body() {
        tokio::time::timeout(Duration::from_secs(5), async {
            let body = serde_json::json!({
                "authorization": SECRET,
                "error": format!("provider echoed {SECRET}"),
            })
            .to_string();

            for operation in ["create session", "read file", "write file"] {
                let (response, server) = response("401 Unauthorized", &body, true).await;
                let result = match operation {
                    "create session" => decode_response::<serde_json::Value>(response, operation)
                        .await
                        .map(|_| ()),
                    "read file" => decode_bytes(response, operation).await.map(|_| ()),
                    _ => decode_empty(response, operation).await,
                };
                server.abort();
                let error = format!("{:#}", result.unwrap_err());
                assert_eq!(error, format!("{operation} failed with 401 Unauthorized"));
                assert!(!error.contains(SECRET));
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn malformed_success_response_cannot_leak_values_through_decode_error() {
        #[derive(Deserialize)]
        struct Payload {
            #[serde(rename = "cancelled")]
            _cancelled: bool,
        }

        let body = serde_json::json!({"cancelled": SECRET}).to_string();
        let (response, server) = response("200 OK", &body, false).await;
        let error = decode_response::<Payload>(response, "cancel command").await;
        let error = format!("{:#}", error.err().expect("expected a schema error"));
        assert!(error.contains("decode cancel command response: invalid payload"));
        assert!(!error.contains(SECRET));
        server.await.unwrap();
    }
}
