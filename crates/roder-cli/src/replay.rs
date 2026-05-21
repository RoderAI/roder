use std::{
    io::Cursor,
    path::{Path, PathBuf},
    sync::Arc,
};

use roder_api_transcript::{ApiTranscriptReader, ApiTranscriptRecord};
use roder_app_server::{AppServer, LocalAppClient};
use roder_protocol::JsonRpcRequest;
use roder_tui::replay::{HeadlessReplayDriver, ReplayTranscript};

use crate::{CliOptions, build_runtime_from_config};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReplaySpeed {
    Real,
    Fast,
    Step,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReplayCliOptions {
    pub path: PathBuf,
    pub headless: bool,
    pub update_frames: bool,
    pub speed: ReplaySpeed,
    pub live: bool,
}

pub(crate) async fn run_replay_cli(args: &[String]) -> anyhow::Result<()> {
    let options = parse_replay_cli_options(args)?;
    if options.live && std::env::var_os("RODER_LIVE_REPLAY").is_none() {
        anyhow::bail!("live replay requires RODER_LIVE_REPLAY=1");
    }

    let records = read_transcript_records(&options.path)?;
    if options.live {
        let replayed = run_live_replay(&records).await?;
        println!(
            "Live replay OK: {} ({} request{})",
            options.path.display(),
            replayed,
            if replayed == 1 { "" } else { "s" },
        );
        return Ok(());
    }

    let transcript = ReplayTranscript::from_records(records.clone())?;
    let frames = transcript.frames().clone();
    if options.headless || !options.live {
        run_headless_frame_replay(records, frames)?;
    }
    println!(
        "Replay OK: {} ({:?}, {} input{}, side-effect-free)",
        options.path.display(),
        options.speed,
        transcript.inputs().len(),
        if transcript.inputs().len() == 1 {
            ""
        } else {
            "s"
        },
    );
    Ok(())
}

async fn run_live_replay(records: &[ApiTranscriptRecord]) -> anyhow::Result<usize> {
    let (runtime, _) = build_runtime_from_config(CliOptions::default()).await?;
    let client = LocalAppClient::new(Arc::new(AppServer::new(runtime)));
    let mut replayed = 0;
    for record in records {
        let ApiTranscriptRecord::ApiRequest { request, .. } = record else {
            continue;
        };
        let request = serde_json::from_value::<JsonRpcRequest>(request.clone())?;
        let response = client.send_request(request).await;
        if let Some(error) = response.error {
            anyhow::bail!(
                "live replay request failed: {} ({})",
                error.message,
                error.code
            );
        }
        replayed += 1;
    }
    Ok(replayed)
}

pub(crate) fn parse_replay_cli_options(args: &[String]) -> anyhow::Result<ReplayCliOptions> {
    let Some(path) = args.first() else {
        anyhow::bail!(
            "usage: roder replay <path> [--headless] [--update-frames] [--speed real|fast|step] [--live]"
        );
    };
    let mut options = ReplayCliOptions {
        path: PathBuf::from(path),
        headless: false,
        update_frames: false,
        speed: ReplaySpeed::Fast,
        live: false,
    };

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--headless" => options.headless = true,
            "--update-frames" => options.update_frames = true,
            "--live" => options.live = true,
            "--speed" => {
                let Some(speed) = args.get(i + 1) else {
                    anyhow::bail!("--speed requires real, fast, or step");
                };
                options.speed = parse_speed(speed)?;
                i += 1;
            }
            arg if arg.starts_with("--speed=") => {
                options.speed = parse_speed(&arg["--speed=".len()..])?;
            }
            other => anyhow::bail!("unknown replay option: {other}"),
        }
        i += 1;
    }
    Ok(options)
}

pub(crate) fn read_transcript_records(path: &Path) -> anyhow::Result<Vec<ApiTranscriptRecord>> {
    let bytes = std::fs::read(path)?;
    ApiTranscriptReader::new(Cursor::new(bytes)).read_records()
}

fn run_headless_frame_replay(
    records: Vec<ApiTranscriptRecord>,
    frames: std::collections::VecDeque<roder_api_transcript::RecordedFrame>,
) -> anyhow::Result<()> {
    let terminal = records
        .iter()
        .find_map(|record| match record {
            ApiTranscriptRecord::Header(header) => Some(header.terminal),
            _ => None,
        })
        .unwrap_or(roder_api_transcript::RecordedTerminalSize { cols: 80, rows: 24 });
    let mut driver = HeadlessReplayDriver::new(terminal, frames);
    for frame in records.iter().filter_map(|record| match record {
        ApiTranscriptRecord::UiFrame { frame, .. } => Some(frame.clone()),
        _ => None,
    }) {
        let text = frame.text.clone().unwrap_or_default();
        driver.draw_and_assert_next_frame(|render_frame| {
            render_frame.render_widget(
                ratatui::widgets::Paragraph::new(text),
                ratatui::layout::Rect::new(0, 0, frame.cols, frame.rows),
            );
        })?;
    }
    Ok(())
}

fn parse_speed(value: &str) -> anyhow::Result<ReplaySpeed> {
    match value {
        "real" => Ok(ReplaySpeed::Real),
        "fast" => Ok(ReplaySpeed::Fast),
        "step" => Ok(ReplaySpeed::Step),
        _ => anyhow::bail!("--speed must be real, fast, or step"),
    }
}

#[cfg(test)]
mod tests {
    use roder_api_transcript::{
        ApiTranscriptHeader, RecordedFrame, RecordedTerminalSize, SUPPORTED_SCHEMA_VERSION,
        write_jsonl_record,
    };
    use time::OffsetDateTime;

    use super::*;

    #[test]
    fn parses_headless_replay_options() {
        let options = parse_replay_cli_options(&[
            "tests/fixtures/api-transcripts/startup.jsonl".to_string(),
            "--headless".to_string(),
            "--update-frames".to_string(),
            "--speed=step".to_string(),
        ])
        .unwrap();

        assert_eq!(
            options.path,
            PathBuf::from("tests/fixtures/api-transcripts/startup.jsonl")
        );
        assert!(options.headless);
        assert!(options.update_frames);
        assert_eq!(options.speed, ReplaySpeed::Step);
    }

    #[test]
    fn parses_live_replay_gate_option() {
        let options = parse_replay_cli_options(&[
            "tests/fixtures/api-transcripts/startup.jsonl".to_string(),
            "--live".to_string(),
            "--speed".to_string(),
            "real".to_string(),
        ])
        .unwrap();

        assert!(options.live);
        assert_eq!(options.speed, ReplaySpeed::Real);
    }

    #[test]
    fn fixture_loading_reads_jsonl_records() {
        let dir = std::env::temp_dir().join(format!("roder-replay-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fixture.jsonl");
        let mut bytes = Vec::new();
        write_jsonl_record(
            &mut bytes,
            &ApiTranscriptRecord::Header(ApiTranscriptHeader {
                schema_version: SUPPORTED_SCHEMA_VERSION,
                created_at: OffsetDateTime::UNIX_EPOCH,
                roder_version: "dev".to_string(),
                cwd: "<redacted>".to_string(),
                terminal: RecordedTerminalSize { cols: 20, rows: 2 },
                features: Vec::new(),
                metadata: serde_json::Value::Null,
            }),
        )
        .unwrap();
        write_jsonl_record(
            &mut bytes,
            &ApiTranscriptRecord::UiFrame {
                seq: 1,
                at_ms: 0,
                frame: RecordedFrame {
                    cols: 20,
                    rows: 2,
                    text_hash: roder_tui::frame_snapshot::frame_text_hash("ready"),
                    text: Some("ready".to_string()),
                    artifacts: Vec::new(),
                },
            },
        )
        .unwrap();
        std::fs::write(&path, bytes).unwrap();

        let records = read_transcript_records(&path).unwrap();

        assert_eq!(records.len(), 2);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn checked_in_startup_fixture_loads() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("tests/fixtures/api-transcripts/startup.jsonl");
        let records = read_transcript_records(&path).unwrap();

        assert_eq!(records.len(), 2);
    }
}
