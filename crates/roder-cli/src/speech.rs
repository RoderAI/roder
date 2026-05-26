use std::path::Path;
use std::sync::Arc;

use base64::Engine;
use roder_app_server::{AppServer, LocalAppClient};
use roder_protocol::{
    JsonRpcRequest, SpeechAudioPayload, SpeechProvidersListResult, SpeechTranscribeParams,
    SpeechTranscribeResult,
};
use tokio::io::AsyncReadExt;

use crate::{CliOptions, build_runtime_from_config, decode_response};

pub(crate) async fn run_speech_cli(args: &[String]) -> anyhow::Result<()> {
    match args.first().map(String::as_str) {
        Some("providers" | "list") => run_speech_providers(&args[1..]).await,
        Some("transcribe") => run_speech_transcribe(&args[1..]).await,
        _ => anyhow::bail!(
            "usage: roder speech providers [--json]\n       roder speech transcribe <audio-file|-> [--provider <id>] [--model <id>] [--language <code>] [--diarization] [--format text|json]"
        ),
    }
}

async fn run_speech_providers(args: &[String]) -> anyhow::Result<()> {
    let json = args.iter().any(|arg| arg == "--json");
    let client = local_client().await?;
    let result: SpeechProvidersListResult = decode_response(
        client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!(1)),
                method: "speech/providers/list".to_string(),
                params: None,
            })
            .await,
    )?;

    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        for provider in result.providers {
            let models = provider
                .models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>()
                .join(",");
            println!(
                "{}\t{}\tauth={}\t{}",
                provider.id, provider.name, provider.authenticated, models
            );
        }
    }
    Ok(())
}

async fn run_speech_transcribe(args: &[String]) -> anyhow::Result<()> {
    let options = TranscribeOptions::parse(args)?;
    let (bytes, mime_type, filename) = read_audio_payload(&options.input).await?;
    let client = local_client().await?;
    let result: SpeechTranscribeResult = decode_response(
        client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!(1)),
                method: "speech/transcribe".to_string(),
                params: Some(serde_json::to_value(SpeechTranscribeParams {
                    provider: options.provider,
                    model: options.model,
                    audio: SpeechAudioPayload {
                        bytes_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
                        mime_type,
                        filename,
                    },
                    language: options.language,
                    prompt: None,
                    diarization: options.diarization,
                    metadata: Default::default(),
                })?),
            })
            .await,
    )?;

    if options.format == OutputFormat::Json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("{}", result.text);
    }
    Ok(())
}

async fn local_client() -> anyhow::Result<LocalAppClient> {
    let (runtime, _) = build_runtime_from_config(CliOptions::default()).await?;
    Ok(LocalAppClient::new(Arc::new(AppServer::new(runtime))))
}

async fn read_audio_payload(input: &str) -> anyhow::Result<(Vec<u8>, String, Option<String>)> {
    if input == "-" {
        let mut bytes = Vec::new();
        tokio::io::stdin().read_to_end(&mut bytes).await?;
        return Ok((bytes, "application/octet-stream".to_string(), None));
    }
    let path = Path::new(input);
    let bytes = tokio::fs::read(path).await?;
    let mime_type = audio_mime_type(path);
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(ToString::to_string);
    Ok((bytes, mime_type, filename))
}

fn audio_mime_type(path: &Path) -> String {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("wav") => "audio/wav",
        Some("mp3") => "audio/mpeg",
        Some("m4a") => "audio/mp4",
        Some("flac") => "audio/flac",
        Some("webm") => "audio/webm",
        Some("ogg") | Some("oga") => "audio/ogg",
        Some("opus") => "audio/opus",
        _ => "application/octet-stream",
    }
    .to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TranscribeOptions {
    input: String,
    provider: Option<String>,
    model: Option<String>,
    language: Option<String>,
    diarization: bool,
    format: OutputFormat,
}

impl TranscribeOptions {
    fn parse(args: &[String]) -> anyhow::Result<Self> {
        let mut input = None;
        let mut provider = None;
        let mut model = None;
        let mut language = None;
        let mut diarization = false;
        let mut format = OutputFormat::Text;
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--provider" => {
                    provider = Some(required_value(args, &mut i, "--provider")?);
                }
                "--model" => {
                    model = Some(required_value(args, &mut i, "--model")?);
                }
                "--language" => {
                    language = Some(required_value(args, &mut i, "--language")?);
                }
                "--format" => {
                    format = match required_value(args, &mut i, "--format")?.as_str() {
                        "text" => OutputFormat::Text,
                        "json" => OutputFormat::Json,
                        other => {
                            anyhow::bail!("unsupported --format {other:?}; expected text or json")
                        }
                    };
                }
                "--diarization" => {
                    diarization = true;
                }
                "--help" | "-h" => {
                    anyhow::bail!(
                        "usage: roder speech transcribe <audio-file|-> [--provider <id>] [--model <id>] [--language <code>] [--diarization] [--format text|json]"
                    );
                }
                arg if arg.starts_with('-') && arg != "-" => {
                    anyhow::bail!("unknown speech transcribe option {arg:?}");
                }
                arg => {
                    if input.replace(arg.to_string()).is_some() {
                        anyhow::bail!("speech transcribe accepts exactly one audio input");
                    }
                }
            }
            i += 1;
        }
        let Some(input) = input else {
            anyhow::bail!("roder speech transcribe requires an audio file path or - for stdin");
        };
        Ok(Self {
            input,
            provider,
            model,
            language,
            diarization,
            format,
        })
    }
}

fn required_value(args: &[String], index: &mut usize, flag: &str) -> anyhow::Result<String> {
    let Some(value) = args.get(*index + 1) else {
        anyhow::bail!("{flag} requires a value");
    };
    *index += 1;
    Ok(value.clone())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputFormat {
    Text,
    Json,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_transcribe_options() {
        let options = TranscribeOptions::parse(&[
            "clip.wav".to_string(),
            "--provider".to_string(),
            "openai-speech".to_string(),
            "--model".to_string(),
            "gpt-4o-mini-transcribe".to_string(),
            "--language".to_string(),
            "en".to_string(),
            "--format".to_string(),
            "json".to_string(),
        ])
        .unwrap();

        assert_eq!(options.input, "clip.wav");
        assert_eq!(options.provider.as_deref(), Some("openai-speech"));
        assert_eq!(options.model.as_deref(), Some("gpt-4o-mini-transcribe"));
        assert_eq!(options.language.as_deref(), Some("en"));
        assert_eq!(options.format, OutputFormat::Json);
    }

    #[test]
    fn maps_audio_mime_types() {
        assert_eq!(audio_mime_type(Path::new("clip.wav")), "audio/wav");
        assert_eq!(audio_mime_type(Path::new("clip.mp3")), "audio/mpeg");
        assert_eq!(
            audio_mime_type(Path::new("clip.unknown")),
            "application/octet-stream"
        );
    }
}
