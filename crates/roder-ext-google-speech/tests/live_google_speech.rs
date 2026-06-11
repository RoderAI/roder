//! Opt-in live Google Speech-to-Text validation (roadmap phase 69). Ignored
//! by default; runs only with `RODER_GOOGLE_SPEECH_LIVE=1` plus either an
//! OAuth access token (`RODER_GOOGLE_SPEECH_ACCESS_TOKEN` and
//! `RODER_GOOGLE_SPEECH_PROJECT`) or an API key
//! (`RODER_GOOGLE_SPEECH_API_KEY`/`GOOGLE_API_KEY`). The audio fixture is
//! synthesized in-process (see `tests/fixtures/audio/README.md`).

use roder_api::speech::{
    SpeechAudio, SpeechProviderContext, SpeechTranscriber, SpeechTranscriptionRequest,
};
use roder_ext_google_speech::{
    GOOGLE_SPEECH_PROVIDER_ID, GoogleSpeechConfig, GoogleSpeechTranscriber,
};

/// Builds a small valid 16-bit mono PCM WAV (440 Hz sine, 0.4 s, 8 kHz).
fn synthetic_wav() -> Vec<u8> {
    let sample_rate = 8_000u32;
    let samples = (sample_rate as f32 * 0.4) as u32;
    let data_len = samples * 2;
    let mut wav = Vec::with_capacity((44 + data_len) as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for index in 0..samples {
        let t = index as f32 / sample_rate as f32;
        let sample = (16_000.0 * (2.0 * std::f32::consts::PI * 440.0 * t).sin()) as i16;
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    wav
}

#[tokio::test]
#[ignore = "requires RODER_GOOGLE_SPEECH_LIVE=1 and Google speech credentials"]
async fn live_google_speech_transcribes_synthetic_wav() {
    if std::env::var("RODER_GOOGLE_SPEECH_LIVE").ok().as_deref() != Some("1") {
        eprintln!("set RODER_GOOGLE_SPEECH_LIVE=1 to run live Google speech tests");
        return;
    }
    let config = GoogleSpeechConfig::from_env();
    assert!(
        config.access_token.is_some() || config.api_key.is_some(),
        "live Google speech tests require RODER_GOOGLE_SPEECH_ACCESS_TOKEN or an API key"
    );

    let provider = GoogleSpeechTranscriber::new(config);
    let result = provider
        .transcribe(
            SpeechProviderContext {
                provider_id: GOOGLE_SPEECH_PROVIDER_ID,
            },
            SpeechTranscriptionRequest {
                model: std::env::var("RODER_GOOGLE_SPEECH_MODEL")
                    .unwrap_or_else(|_| "latest_short".to_string()),
                audio: SpeechAudio {
                    bytes: synthetic_wav(),
                    mime_type: "audio/wav".to_string(),
                    filename: Some("tone.wav".to_string()),
                },
                language: Some("en-US".to_string()),
                prompt: None,
                diarization: false,
                metadata: Default::default(),
            },
        )
        .await
        .expect("live Google speech transcription request should succeed");

    // A pure sine tone has no guaranteed transcript; the live check validates
    // auth, request mapping, and response parsing rather than exact text.
    eprintln!("live Google speech transcript: {:?}", result.text);
}
