//! Offline fake-HTTP coverage for the OpenAI speech provider: multipart body
//! shape, successful response parsing, and provider error surfacing
//! (roadmap phase 69). No network access or credentials are required.

use roder_api::speech::{
    SpeechAudio, SpeechProviderContext, SpeechTranscriber, SpeechTranscriptionRequest,
};
use roder_ext_openai_speech::{
    OPENAI_SPEECH_PROVIDER_ID, OpenAiSpeechConfig, OpenAiSpeechTranscriber,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

/// One-shot fake HTTP server: captures the raw request (headers + body) and
/// answers with the given status and JSON body.
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

fn provider_for(base_url: &str) -> OpenAiSpeechTranscriber {
    OpenAiSpeechTranscriber::new(
        OpenAiSpeechConfig::new(Some("test-secret".to_string())).with_base_url(base_url),
    )
}

fn request(model: &str, diarization: bool) -> SpeechTranscriptionRequest {
    SpeechTranscriptionRequest {
        model: model.to_string(),
        audio: SpeechAudio {
            bytes: b"RIFF-fake-audio-bytes".to_vec(),
            mime_type: "audio/wav".to_string(),
            filename: Some("clip.wav".to_string()),
        },
        language: Some("en".to_string()),
        prompt: Some("transcribe the greeting".to_string()),
        diarization,
        metadata: Default::default(),
    }
}

fn ctx() -> SpeechProviderContext<'static> {
    SpeechProviderContext {
        provider_id: OPENAI_SPEECH_PROVIDER_ID,
    }
}

#[tokio::test]
async fn multipart_body_carries_model_language_prompt_and_audio_file() {
    let (base_url, mut captured) = spawn_one_shot_server(
        200,
        r#"{"text":"hello world","language":"en","duration":1.5}"#,
    )
    .await;
    let provider = provider_for(&base_url);

    let result = provider
        .transcribe(ctx(), request("gpt-4o-mini-transcribe", false))
        .await
        .unwrap();

    assert_eq!(result.text, "hello world");
    assert_eq!(result.language.as_deref(), Some("en"));
    assert_eq!(result.duration_millis, Some(1500));

    let raw = captured.recv().await.unwrap();
    assert!(raw.starts_with("POST /audio/transcriptions"), "{raw}");
    assert!(raw.contains("authorization: Bearer test-secret"), "{raw}");
    assert!(raw.contains("name=\"model\""), "{raw}");
    assert!(raw.contains("gpt-4o-mini-transcribe"), "{raw}");
    assert!(raw.contains("name=\"response_format\""), "{raw}");
    assert!(raw.contains("name=\"language\""), "{raw}");
    assert!(raw.contains("name=\"prompt\""), "{raw}");
    assert!(raw.contains("transcribe the greeting"), "{raw}");
    assert!(
        raw.contains("name=\"file\"; filename=\"clip.wav\""),
        "{raw}"
    );
    assert!(raw.contains("Content-Type: audio/wav"), "{raw}");
    assert!(raw.contains("RIFF-fake-audio-bytes"), "{raw}");
    assert!(
        !raw.contains("chunking_strategy"),
        "non-diarize requests must not set chunking_strategy: {raw}"
    );
}

#[tokio::test]
async fn diarize_request_sets_chunking_strategy_and_maps_segments() {
    let (base_url, mut captured) = spawn_one_shot_server(
        200,
        r#"{
            "text":"hi there",
            "segments":[
                {"text":"hi","start":0.0,"end":0.4,"speaker":"A","confidence":0.93},
                {"text":"there","start":0.4,"end":0.9,"speaker":"B"}
            ]
        }"#,
    )
    .await;
    let provider = provider_for(&base_url);

    let result = provider
        .transcribe(ctx(), request("gpt-4o-transcribe-diarize", true))
        .await
        .unwrap();

    assert_eq!(result.text, "hi there");
    assert_eq!(result.segments.len(), 2);
    assert_eq!(result.segments[0].speaker.as_deref(), Some("A"));
    assert_eq!(result.segments[0].start_millis, Some(0));
    assert_eq!(result.segments[0].end_millis, Some(400));
    assert_eq!(result.segments[0].confidence, Some(0.93));
    assert_eq!(result.segments[1].speaker.as_deref(), Some("B"));

    let raw = captured.recv().await.unwrap();
    assert!(raw.contains("name=\"chunking_strategy\""), "{raw}");
    assert!(raw.contains("auto"), "{raw}");
}

#[tokio::test]
async fn provider_error_response_surfaces_status_and_body() {
    let (base_url, _captured) = spawn_one_shot_server(
        400,
        r#"{"error":{"message":"Unsupported audio format","type":"invalid_request_error"}}"#,
    )
    .await;
    let provider = provider_for(&base_url);

    let error = provider
        .transcribe(ctx(), request("whisper-1", false))
        .await
        .unwrap_err()
        .to_string();

    assert!(error.contains("400"), "{error}");
    assert!(error.contains("Unsupported audio format"), "{error}");
}
