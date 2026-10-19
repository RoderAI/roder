use roder_api::remote_runner::{RunnerDestination, RunnerManifest};
use roder_ext_runner_blaxel::{BlaxelClient, BlaxelConfig, PROVIDER_ID};
use serde_json::json;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

#[tokio::test]
async fn binary_files_preserve_all_byte_values_without_json_or_utf8_conversion() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let bytes = (0..=255u8).collect::<Vec<_>>();
    let response_bytes = bytes.clone();
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        for response in [b"{}".as_slice(), response_bytes.as_slice()] {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut chunk = [0; 4096];
                let read = socket.read(&mut chunk).await.unwrap();
                assert!(read > 0);
                request.extend_from_slice(&chunk[..read]);
                if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                    let length = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .unwrap_or("0")
                        .parse::<usize>()
                        .unwrap();
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            // Intentionally no Content-Type: the read contract is bytes.
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        response.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            socket.write_all(response).await.unwrap();
            requests.push(request);
        }
        requests
    });
    let config = BlaxelConfig::from_destination(&RunnerDestination {
        id: "local".into(),
        provider_id: PROVIDER_ID.into(),
        default_manifest: RunnerManifest::default(),
        config: json!({"token":"test-token","workspace":"test-workspace","cleanup":"keep"}),
    })
    .unwrap();
    let client = BlaxelClient::new(reqwest::Client::new(), &config);
    client
        .write_file(&endpoint, "/tmp/binary payload.bin", &bytes)
        .await
        .unwrap();
    assert_eq!(
        client
            .read_file(&endpoint, "/tmp/binary payload.bin")
            .await
            .unwrap(),
        bytes
    );
    let requests = server.await.unwrap();
    let put = String::from_utf8_lossy(&requests[0]);
    assert!(put.starts_with("PUT ") && put.contains("binary%20payload.bin"));
    assert!(
        put.to_ascii_lowercase()
            .contains("content-type: multipart/form-data; boundary=")
    );
    assert!(put.contains("name=\"permissions\"\r\n\r\n0600"));
    assert!(requests[0].windows(bytes.len()).any(|part| part == bytes));
    let get = String::from_utf8_lossy(&requests[1]);
    assert!(
        get.starts_with("GET ")
            && get
                .to_ascii_lowercase()
                .contains("accept: application/octet-stream")
    );
}
