//! Offline fake-HTTP coverage for the Google Speech-to-Text provider:
//! v2 recognize request shape, response mapping (transcripts, confidence,
//! word timestamps, speaker labels, language), and error surfacing
//! (roadmap phase 69). No network access or credentials are required.

use roder_api::speech::{
    SpeechAudio, SpeechProviderContext, SpeechTranscriber, SpeechTranscriptionRequest,
};
use roder_ext_google_speech::{
    GOOGLE_SPEECH_PROVIDER_ID, GoogleSpeechConfig, GoogleSpeechTranscriber,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

async fn spawn_one_shot_server(
    status: u16,
    body: &'static str,
) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel(1);
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut raw = Vec::new();
        let mut buffer = [0u8; 8192];
        loop {
            let read = stream.read(&mut buffer).await.unwrap();
            if read == 0 {
                break;
            }
            raw.extend_from_slice(&buffer[..read]);
            let text = String::from_utf8_lossy(&raw);
            if let Some(header_end) = text.find("\r\n\r\n") {
                let content_length = text
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|value| value.trim().parse::<usize>().unwrap_or(0))
                    })
                    .unwrap_or(0);
                if raw.len() >= header_end + 4 + content_length {
                    break;
                }
            }
        }
        let reason = if status < 400 { "OK" } else { "Bad Request" };
        let response = format!(
            "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).await.unwrap();
        let _ = stream.shutdown().await;
        let _ = tx.send(String::from_utf8_lossy(&raw).into_owned()).await;
    });
    (base_url, rx)
}

fn provider_for(endpoint: &str) -> GoogleSpeechTranscriber {
    GoogleSpeechTranscriber::new(GoogleSpeechConfig {
        access_token: Some("test-oauth-token".to_string()),
        api_key: None,
        project_id: Some("test-project".to_string()),
        location: "global".to_string(),
        endpoint: endpoint.to_string(),
    })
}

fn request(diarization: bool) -> SpeechTranscriptionRequest {
    SpeechTranscriptionRequest {
        model: "latest_short".to_string(),
        audio: SpeechAudio {
            bytes: b"fake-pcm-bytes".to_vec(),
            mime_type: "audio/wav".to_string(),
            filename: Some("clip.wav".to_string()),
        },
        language: Some("en-US".to_string()),
        prompt: None,
        diarization,
        metadata: Default::default(),
    }
}

fn ctx() -> SpeechProviderContext<'static> {
    SpeechProviderContext {
        provider_id: GOOGLE_SPEECH_PROVIDER_ID,
    }
}

#[tokio::test]
async fn v2_recognize_request_maps_config_content_and_response_fields() {
    let (endpoint, mut captured) = spawn_one_shot_server(
        200,
        r#"{
            "results": [
                {
                    "alternatives": [
                        {
                            "transcript": "hello world",
                            "confidence": 0.91,
                            "words": [
                                {"word":"hello","startOffset":"0s","endOffset":"0.500s","speakerLabel":"1"},
                                {"word":"world","startOffset":"0.500s","endOffset":"1s","speakerLabel":"2"}
                            ]
                        }
                    ],
                    "languageCode": "en-US"
                }
            ]
        }"#,
    )
    .await;
    let provider = provider_for(&endpoint);

    let result = provider.transcribe(ctx(), request(true)).await.unwrap();

    assert_eq!(result.text, "hello world");
    assert_eq!(result.language.as_deref(), Some("en-US"));
    assert_eq!(result.segments.len(), 3);
    assert_eq!(result.segments[0].confidence, Some(0.91));
    assert_eq!(result.segments[1].text, "hello");
    assert_eq!(result.segments[1].start_millis, Some(0));
    assert_eq!(result.segments[1].end_millis, Some(500));
    assert_eq!(result.segments[1].speaker.as_deref(), Some("1"));
    assert_eq!(result.segments[2].speaker.as_deref(), Some("2"));

    let raw = captured.recv().await.unwrap();
    assert!(
        raw.starts_with("POST /v2/projects/test-project/locations/global/recognizers/_:recognize"),
        "{raw}"
    );
    assert!(
        raw.contains("authorization: Bearer test-oauth-token"),
        "{raw}"
    );
    let body_start = raw.find("\r\n\r\n").unwrap() + 4;
    let body: serde_json::Value = serde_json::from_str(&raw[body_start..]).unwrap();
    assert_eq!(body["config"]["model"], "latest_short");
    assert_eq!(body["config"]["languageCodes"][0], "en-US");
    assert_eq!(body["config"]["features"]["enableWordTimeOffsets"], true);
    assert!(body["config"]["features"]["diarizationConfig"].is_object());
    // Audio bytes travel base64-encoded in `content`.
    assert_eq!(
        body["content"],
        serde_json::Value::String("ZmFrZS1wY20tYnl0ZXM=".to_string())
    );
}

#[tokio::test]
async fn provider_error_response_surfaces_status_and_body() {
    let (endpoint, _captured) = spawn_one_shot_server(
        400,
        r#"{"error":{"code":400,"message":"Invalid recognizer","status":"INVALID_ARGUMENT"}}"#,
    )
    .await;
    let provider = provider_for(&endpoint);

    let error = provider
        .transcribe(ctx(), request(false))
        .await
        .unwrap_err()
        .to_string();

    assert!(error.contains("400"), "{error}");
    assert!(error.contains("Invalid recognizer"), "{error}");
}
