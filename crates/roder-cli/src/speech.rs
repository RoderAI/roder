use std::path::Path;
use std::sync::Arc;

use base64::Engine;
use roder_app_server::{AppServer, LocalAppClient};
use roder_protocol::{
    JsonRpcRequest, SpeechAudioPayload, SpeechProvidersListResult,
    SpeechSynthesisProvidersListResult, SpeechSynthesizeParams, SpeechSynthesizeResult,
    SpeechTranscribeParams, SpeechTranscribeResult,
};
use tokio::io::AsyncReadExt;

use crate::{CliOptions, build_runtime_from_config, decode_response};

pub(crate) async fn run_speech_cli(args: &[String]) -> anyhow::Result<()> {
    match args.first().map(String::as_str) {
        Some("providers" | "list") => run_speech_providers(&args[1..]).await,
        Some("synthesis-providers" | "synthesizers") => {
            run_speech_synthesis_providers(&args[1..]).await
        }
        Some("synthesize") => run_speech_synthesize(&args[1..]).await,
        Some("transcribe") => run_speech_transcribe(&args[1..]).await,
        _ => anyhow::bail!(
            "usage: roder speech providers [--json]\n       roder speech synthesis-providers [--json]\n       roder speech synthesize <text> [--provider <id>] [--model <id>] [--voice <id>] [--audio-format wav|pcm16] [--prompt <text>] [--output <path>] [--format json]\n       roder speech transcribe <audio-file|-> [--provider <id>] [--model <id>] [--language <code>] [--diarization] [--format text|json] [--to-thread <thread-id>]"
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

async fn run_speech_synthesis_providers(args: &[String]) -> anyhow::Result<()> {
    let json = args.iter().any(|arg| arg == "--json");
    let client = local_client().await?;
    let result: SpeechSynthesisProvidersListResult = decode_response(
        client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!(1)),
                method: "speech/synthesis/providers/list".to_string(),
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

    // Explicit opt-in only: turns are never started from audio silently.
    if let Some(thread_id) = options.to_thread {
        anyhow::ensure!(
            !result.text.trim().is_empty(),
            "transcript is empty; not starting a turn on thread {thread_id}"
        );
        let started: roder_protocol::TurnStartResult = decode_response(
            client
                .send_request(JsonRpcRequest {
                    jsonrpc: "2.0".to_string(),
                    id: Some(serde_json::json!(2)),
                    method: "turn/start".to_string(),
                    params: Some(serde_json::json!({
                        "threadId": thread_id,
                        "prompt": result.text,
                    })),
                })
                .await,
        )?;
        eprintln!("started turn {} on thread {thread_id} from the transcript", started.turn_id);
    }
    Ok(())
}

async fn run_speech_synthesize(args: &[String]) -> anyhow::Result<()> {
    let options = SynthesizeOptions::parse(args)?;
    let client = local_client().await?;
    let result: SpeechSynthesizeResult = decode_response(
        client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!(1)),
                method: "speech/synthesize".to_string(),
                params: Some(serde_json::to_value(SpeechSynthesizeParams {
                    provider: options.provider,
                    model: options.model,
                    text: options.text,
                    voice: options.voice,
                    audio_format: options.audio_format,
                    prompt: options.prompt,
                    voice_sample: None,
                    metadata: Default::default(),
                })?),
            })
            .await,
    )?;

    if options.format == OutputFormat::Json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else if let Some(output) = options.output {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(result.audio.bytes_base64.as_bytes())?;
        tokio::fs::write(output, bytes).await?;
    } else {
        println!("{}", result.audio.bytes_base64);
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
    /// Send the final transcript into `turn/start` on this thread. Turns
    /// are never started from audio without this explicit flag.
    to_thread: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SynthesizeOptions {
    text: String,
    provider: Option<String>,
    model: Option<String>,
    voice: Option<String>,
    audio_format: Option<String>,
    prompt: Option<String>,
    output: Option<String>,
    format: OutputFormat,
}

impl SynthesizeOptions {
    fn parse(args: &[String]) -> anyhow::Result<Self> {
        let mut text = None;
        let mut provider = None;
        let mut model = None;
        let mut voice = None;
        let mut audio_format = None;
        let mut prompt = None;
        let mut output = None;
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
                "--voice" => {
                    voice = Some(required_value(args, &mut i, "--voice")?);
                }
                "--audio-format" => {
                    audio_format = Some(required_value(args, &mut i, "--audio-format")?);
                }
                "--prompt" => {
                    prompt = Some(required_value(args, &mut i, "--prompt")?);
                }
                "--output" => {
                    output = Some(required_value(args, &mut i, "--output")?);
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
                "--help" | "-h" => {
                    anyhow::bail!(
                        "usage: roder speech synthesize <text> [--provider <id>] [--model <id>] [--voice <id>] [--audio-format wav|pcm16] [--prompt <text>] [--output <path>] [--format json]"
                    );
                }
                arg if arg.starts_with('-') => {
                    anyhow::bail!("unknown speech synthesize option {arg:?}");
                }
                arg => {
                    if text.replace(arg.to_string()).is_some() {
                        anyhow::bail!("speech synthesize accepts exactly one text argument");
                    }
                }
            }
            i += 1;
        }
        let Some(text) = text else {
            anyhow::bail!("roder speech synthesize requires text");
        };
        Ok(Self {
            text,
            provider,
            model,
            voice,
            audio_format,
            prompt,
            output,
            format,
        })
    }
}

impl TranscribeOptions {
    fn parse(args: &[String]) -> anyhow::Result<Self> {
        let mut input = None;
        let mut provider = None;
        let mut model = None;
        let mut language = None;
        let mut diarization = false;
        let mut format = OutputFormat::Text;
        let mut to_thread = None;
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
                "--to-thread" => {
                    to_thread = Some(required_value(args, &mut i, "--to-thread")?);
                }
                "--help" | "-h" => {
                    anyhow::bail!(
                        "usage: roder speech transcribe <audio-file|-> [--provider <id>] [--model <id>] [--language <code>] [--diarization] [--format text|json] [--to-thread <thread-id>]"
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
            to_thread,
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

    #[test]
    fn transcribe_options_parse_to_thread_flag() {
        let args: Vec<String> = ["clip.wav", "--provider", "fake", "--to-thread", "thread-9"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let options = TranscribeOptions::parse(&args).unwrap();
        assert_eq!(options.input, "clip.wav");
        assert_eq!(options.to_thread.as_deref(), Some("thread-9"));

        // Without the flag, no turn is ever started from audio.
        let args: Vec<String> = vec!["clip.wav".to_string()];
        assert_eq!(TranscribeOptions::parse(&args).unwrap().to_thread, None);
    }
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

    #[test]
    fn parses_synthesize_options() {
        let options = SynthesizeOptions::parse(&[
            "hello".to_string(),
            "--provider".to_string(),
            "xiaomi-mimo".to_string(),
            "--model".to_string(),
            "mimo-v2.5-tts".to_string(),
            "--voice".to_string(),
            "Chloe".to_string(),
            "--audio-format".to_string(),
            "wav".to_string(),
            "--output".to_string(),
            "out.wav".to_string(),
        ])
        .unwrap();

        assert_eq!(options.text, "hello");
        assert_eq!(options.provider.as_deref(), Some("xiaomi-mimo"));
        assert_eq!(options.model.as_deref(), Some("mimo-v2.5-tts"));
        assert_eq!(options.voice.as_deref(), Some("Chloe"));
        assert_eq!(options.audio_format.as_deref(), Some("wav"));
        assert_eq!(options.output.as_deref(), Some("out.wav"));
    }
}
