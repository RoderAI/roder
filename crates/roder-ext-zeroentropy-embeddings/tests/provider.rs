use roder_api::embeddings::{EmbeddingInputType, EmbeddingProvider, EmbeddingRequest};
use roder_ext_zeroentropy_embeddings::{
    DEFAULT_DIMENSIONS, DEFAULT_MODEL, ZeroEntropyEmbeddingProvider, ZeroEntropyEmbeddingsConfig,
    ZeroEntropyEncodingFormat, ZeroEntropyLatency,
};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[test]
fn registers_zeroentropy_embedding_descriptor() {
    let descriptor =
        ZeroEntropyEmbeddingProvider::new(ZeroEntropyEmbeddingsConfig::default()).descriptor();

    assert_eq!(descriptor.id, "zeroentropy");
    assert_eq!(descriptor.default_model, DEFAULT_MODEL);
    assert_eq!(descriptor.models[0].dimensions, DEFAULT_DIMENSIONS);
}

#[tokio::test]
async fn missing_key_returns_concise_error() {
    let err = ZeroEntropyEmbeddingProvider::new(ZeroEntropyEmbeddingsConfig {
        api_key: Some(String::new()),
        ..ZeroEntropyEmbeddingsConfig::default()
    })
    .with_endpoint("http://127.0.0.1:1")
    .embed(EmbeddingRequest {
        model: DEFAULT_MODEL.to_string(),
        inputs: vec!["hello".to_string()],
        input_type: EmbeddingInputType::Document,
        dimensions: None,
    })
    .await
    .unwrap_err();

    assert!(err.to_string().contains("ZeroEntropy embeddings require"));
}

#[tokio::test]
async fn maps_query_request_and_float_response() {
    let server = TestServer::start(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"results\":[{\"embedding\":[0.1,0.2,0.3]}],\"usage\":{\"total_bytes\":123,\"total_tokens\":5}}\n",
        1,
    )
    .await;
    let provider = ZeroEntropyEmbeddingProvider::new(ZeroEntropyEmbeddingsConfig {
        api_key: Some("test-key".to_string()),
        endpoint: server.endpoint(),
        encoding_format: ZeroEntropyEncodingFormat::Float,
        latency: Some(ZeroEntropyLatency::Fast),
    });

    let response = provider
        .embed(EmbeddingRequest {
            model: DEFAULT_MODEL.to_string(),
            inputs: vec!["who owns acme?".to_string()],
            input_type: EmbeddingInputType::Query,
            dimensions: Some(2560),
        })
        .await
        .unwrap();

    assert_eq!(response.provider_id, "zeroentropy");
    assert_eq!(response.model, DEFAULT_MODEL);
    assert_eq!(response.embeddings[0].index, 0);
    assert_eq!(response.embeddings[0].values, vec![0.1, 0.2, 0.3]);
    let request = server.requests().join("\n");
    let request_lower = request.to_ascii_lowercase();
    assert!(request.contains("POST /models/embed"));
    assert!(request_lower.contains("authorization: bearer test-key"));
    assert!(request.contains("\"model\":\"zembed-1\""));
    assert!(request.contains("\"input_type\":\"query\""));
    assert!(request.contains("\"input\":[\"who owns acme?\"]"));
    assert!(request.contains("\"dimensions\":2560"));
    assert!(request.contains("\"encoding_format\":\"float\""));
    assert!(request.contains("\"latency\":\"fast\""));
}

#[tokio::test]
async fn maps_document_request_and_base64_response() {
    let encoded = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        [1.0_f32, -2.0_f32]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    let body = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{{\"results\":[{{\"embedding\":\"{encoded}\"}}],\"usage\":{{\"total_bytes\":123,\"total_tokens\":5}}}}\n"
    );
    let server = TestServer::start(Box::leak(body.into_boxed_str()), 1).await;
    let provider = ZeroEntropyEmbeddingProvider::new(ZeroEntropyEmbeddingsConfig {
        api_key: Some("test-key".to_string()),
        endpoint: server.endpoint(),
        encoding_format: ZeroEntropyEncodingFormat::Base64,
        latency: None,
    });

    let response = provider
        .embed(EmbeddingRequest {
            model: DEFAULT_MODEL.to_string(),
            inputs: vec!["stored fact".to_string()],
            input_type: EmbeddingInputType::Document,
            dimensions: Some(1280),
        })
        .await
        .unwrap();

    assert_eq!(response.embeddings[0].values, vec![1.0, -2.0]);
    let request = server.requests().join("\n");
    assert!(request.contains("\"input_type\":\"document\""));
    assert!(request.contains("\"encoding_format\":\"base64\""));
    assert!(!request.contains("\"latency\""));
}

#[tokio::test]
async fn preserves_input_order_from_results() {
    let server = TestServer::start(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"results\":[{\"embedding\":[0.4]},{\"embedding\":[0.8]}],\"usage\":{\"total_bytes\":123,\"total_tokens\":5}}\n",
        1,
    )
    .await;
    let provider =
        ZeroEntropyEmbeddingProvider::with_api_key("test-key").with_endpoint(server.endpoint());

    let response = provider
        .embed(EmbeddingRequest {
            model: DEFAULT_MODEL.to_string(),
            inputs: vec!["first".to_string(), "second".to_string()],
            input_type: EmbeddingInputType::Document,
            dimensions: Some(80),
        })
        .await
        .unwrap();

    assert_eq!(
        response
            .embeddings
            .iter()
            .map(|embedding| (embedding.index, embedding.values.clone()))
            .collect::<Vec<_>>(),
        vec![(0, vec![0.4]), (1, vec![0.8])]
    );
}

#[tokio::test]
async fn unsupported_dimension_is_reported_before_network() {
    let provider = ZeroEntropyEmbeddingProvider::with_api_key("test-key");

    let err = provider
        .embed(EmbeddingRequest {
            model: DEFAULT_MODEL.to_string(),
            inputs: vec!["hello".to_string()],
            input_type: EmbeddingInputType::Document,
            dimensions: Some(42),
        })
        .await
        .unwrap_err()
        .to_string();

    assert!(err.contains("ZeroEntropy zembed-1 dimensions"));
    assert!(err.contains("2560"));
}

#[tokio::test]
async fn unsupported_model_is_reported_before_network() {
    let provider = ZeroEntropyEmbeddingProvider::with_api_key("test-key");

    let err = provider
        .embed(EmbeddingRequest {
            model: "other-model".to_string(),
            inputs: vec!["hello".to_string()],
            input_type: EmbeddingInputType::Document,
            dimensions: Some(80),
        })
        .await
        .unwrap_err()
        .to_string();

    assert!(err.contains(DEFAULT_MODEL));
}

#[tokio::test]
async fn non_success_status_is_reported() {
    let server = TestServer::start(
        "HTTP/1.1 429 Too Many Requests\r\nContent-Type: application/json\r\n\r\n{\"error\":\"too fast\"}\n",
        1,
    )
    .await;
    let provider =
        ZeroEntropyEmbeddingProvider::with_api_key("test-key").with_endpoint(server.endpoint());

    let err = provider
        .embed(EmbeddingRequest {
            model: DEFAULT_MODEL.to_string(),
            inputs: vec!["hello".to_string()],
            input_type: EmbeddingInputType::Document,
            dimensions: Some(80),
        })
        .await
        .unwrap_err();

    assert!(err.to_string().contains("429 Too Many Requests"));
    assert!(err.to_string().contains("too fast"));
}

struct TestServer {
    addr: SocketAddr,
    requests: Arc<Mutex<Vec<String>>>,
}

impl TestServer {
    async fn start(response: &'static str, expected_requests: usize) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        tokio::spawn(async move {
            for _ in 0..expected_requests {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut buffer = [0_u8; 8192];
                let read = stream.read(&mut buffer).await.unwrap();
                captured
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&buffer[..read]).to_string());
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        Self { addr, requests }
    }

    fn endpoint(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}
