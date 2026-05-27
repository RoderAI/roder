use std::collections::BTreeMap;
use std::ffi::OsString;
use std::process::Stdio;
use std::time::{Duration, Instant};

use base64::Engine;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use roder_app_server::AppClient;
use roder_protocol::{
    JsonRpcRequest, SpeechAudioPayload, SpeechTranscribeParams, SpeechTranscribeResult,
};
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};
use tokio::task::JoinHandle;

use super::{TuiApp, composer_text, composer_textarea, decode_response};

const RECORD_COMMAND_ENV: &str = "RODER_VOICE_RECORD_COMMAND";
const PROVIDER_ENV: &str = "RODER_VOICE_PROVIDER";
const MODEL_ENV: &str = "RODER_VOICE_MODEL";
const LANGUAGE_ENV: &str = "RODER_VOICE_LANGUAGE";
const MIME_TYPE_ENV: &str = "RODER_VOICE_MIME_TYPE";
const HOLD_IDLE_STOP_ENV: &str = "RODER_VOICE_HOLD_IDLE_STOP_MS";
const DEFAULT_MIME_TYPE: &str = "audio/wav";
const DEFAULT_HOLD_IDLE_STOP: Duration = Duration::from_millis(2_000);
const MANUAL_STOP_KEY_GAP: Duration = Duration::from_millis(1_200);
const RECORDING_FINISH_TIMEOUT: Duration = Duration::from_secs(10);
const RECORDER_INTERRUPT_TIMEOUT: Duration = Duration::from_secs(3);
const RECORDER_TERMINATE_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum VoiceMode {
    Hold,
    Tap,
}

#[derive(Debug)]
pub(super) struct VoiceState {
    enabled: bool,
    mode: VoiceMode,
    record_command: Option<String>,
    provider: Option<String>,
    model: Option<String>,
    language: Option<String>,
    mime_type: String,
    hold_idle_stop: Duration,
    recording: Option<VoiceRecording>,
    transcribing: bool,
    transcription_task: Option<JoinHandle<anyhow::Result<String>>>,
    unavailable_reported: bool,
}

#[derive(Debug)]
struct VoiceRecording {
    child: Child,
    started_at: Instant,
    last_space_at: Instant,
    refreshed_by_hold: bool,
}

#[derive(Debug, Clone, Default)]
pub(super) struct VoiceConfig {
    pub(super) enabled: Option<bool>,
    pub(super) mode: Option<VoiceMode>,
    pub(super) record_command: Option<String>,
    pub(super) provider: Option<String>,
    pub(super) model: Option<String>,
    pub(super) language: Option<String>,
    pub(super) mime_type: Option<String>,
    pub(super) hold_idle_stop_millis: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum VoiceKeyAction {
    Pass,
    Consumed,
    Start,
    Stop,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum VoiceCommandAction {
    Toggle,
    Enable(VoiceMode),
    Disable,
    Status,
}

impl Default for VoiceState {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: VoiceMode::Hold,
            record_command: None,
            provider: None,
            model: None,
            language: None,
            mime_type: DEFAULT_MIME_TYPE.to_string(),
            hold_idle_stop: DEFAULT_HOLD_IDLE_STOP,
            recording: None,
            transcribing: false,
            transcription_task: None,
            unavailable_reported: false,
        }
    }
}

impl VoiceState {
    pub(super) fn from_config(config: VoiceConfig) -> Self {
        Self {
            enabled: config.enabled.unwrap_or(false),
            mode: config.mode.unwrap_or(VoiceMode::Hold),
            record_command: non_empty_env(RECORD_COMMAND_ENV).or(config.record_command),
            provider: non_empty_env(PROVIDER_ENV).or(config.provider),
            model: non_empty_env(MODEL_ENV).or(config.model),
            language: non_empty_env(LANGUAGE_ENV).or(config.language),
            mime_type: non_empty_env(MIME_TYPE_ENV)
                .or(config.mime_type)
                .unwrap_or_else(|| DEFAULT_MIME_TYPE.to_string()),
            hold_idle_stop: voice_duration_from_env(HOLD_IDLE_STOP_ENV)
                .or_else(|| config.hold_idle_stop_millis.map(Duration::from_millis))
                .unwrap_or(DEFAULT_HOLD_IDLE_STOP),
            ..Self::default()
        }
    }

    pub(super) fn footer_hint(&self, composer_is_empty: bool) -> Option<String> {
        if !self.enabled {
            return None;
        }
        if self.recording.is_some() {
            return Some("  voice:recording".to_string());
        }
        if self.transcribing {
            return Some("  voice:transcribing".to_string());
        }
        if !composer_is_empty {
            return None;
        }
        Some(match self.mode {
            VoiceMode::Hold => "  hold Space to speak".to_string(),
            VoiceMode::Tap => "  tap Space to speak".to_string(),
        })
    }

    #[cfg(test)]
    pub(super) fn enable_for_test(&mut self) {
        self.enabled = true;
    }

    fn handle_key(&self, key: KeyEvent, composer_is_blank: bool) -> VoiceKeyAction {
        if !self.enabled || key.modifiers != KeyModifiers::NONE || key.code != KeyCode::Char(' ') {
            return VoiceKeyAction::Pass;
        }
        if self.transcribing {
            return VoiceKeyAction::Consumed;
        }

        match self.mode {
            VoiceMode::Hold => match key.kind {
                KeyEventKind::Press if self.should_stop_hold_recording_with_space() => {
                    VoiceKeyAction::Stop
                }
                KeyEventKind::Press | KeyEventKind::Repeat if self.recording.is_some() => {
                    VoiceKeyAction::Consumed
                }
                KeyEventKind::Press | KeyEventKind::Repeat if composer_is_blank => {
                    VoiceKeyAction::Start
                }
                KeyEventKind::Release if self.recording.is_some() => VoiceKeyAction::Stop,
                _ => VoiceKeyAction::Pass,
            },
            VoiceMode::Tap => match key.kind {
                KeyEventKind::Press | KeyEventKind::Repeat if self.recording.is_some() => {
                    VoiceKeyAction::Stop
                }
                KeyEventKind::Press | KeyEventKind::Repeat if composer_is_blank => {
                    VoiceKeyAction::Start
                }
                KeyEventKind::Release if self.recording.is_some() => VoiceKeyAction::Consumed,
                _ => VoiceKeyAction::Pass,
            },
        }
    }

    fn start_recording(&mut self) -> VoiceStartResult {
        if self.recording.is_some() {
            return VoiceStartResult::AlreadyRecording;
        }
        let Some(command) = self.record_command() else {
            return self.unavailable_once(format!(
                "no recorder found; set [tui.voice].record_command in config.toml or {RECORD_COMMAND_ENV} to a command that records audio to stdout"
            ));
        };
        match spawn_recording_command(&command) {
            Ok(child) => {
                self.unavailable_reported = false;
                self.recording = Some(VoiceRecording {
                    child,
                    started_at: Instant::now(),
                    last_space_at: Instant::now(),
                    refreshed_by_hold: false,
                });
                VoiceStartResult::Started
            }
            Err(err) => self.unavailable_once(format!("failed to start voice recorder: {err}")),
        }
    }

    fn take_recording(&mut self) -> Option<VoiceRecording> {
        self.recording.take()
    }

    fn take_idle_hold_recording(&mut self, now: Instant) -> Option<VoiceRecording> {
        if !matches!(self.mode, VoiceMode::Hold) {
            return None;
        }
        let recording = self.recording.as_ref()?;
        if !recording.refreshed_by_hold {
            return None;
        }
        if now.duration_since(recording.last_space_at) < self.hold_idle_stop {
            return None;
        }
        self.recording.take()
    }

    fn command_status(&self) -> String {
        let state = if self.enabled { "enabled" } else { "disabled" };
        let command = self
            .record_command()
            .unwrap_or_else(|| "not configured".to_string());
        format!(
            "Voice mode {state} ({}). Recorder: {command}. Provider: {}. Model: {}. Language: {}.",
            self.mode.label(),
            self.provider.as_deref().unwrap_or("default"),
            self.model.as_deref().unwrap_or("default"),
            self.language.as_deref().unwrap_or("default"),
        )
    }

    fn record_command(&self) -> Option<String> {
        self.record_command.clone().or_else(auto_record_command)
    }

    pub(super) fn is_transcribing(&self) -> bool {
        self.transcribing
    }

    fn start_transcribing(&mut self, task: JoinHandle<anyhow::Result<String>>) {
        self.transcribing = true;
        self.transcription_task = Some(task);
    }

    fn transcription_finished(&self) -> bool {
        self.transcription_task
            .as_ref()
            .is_some_and(JoinHandle::is_finished)
    }

    fn take_transcription_task(&mut self) -> Option<JoinHandle<anyhow::Result<String>>> {
        self.transcription_task.take()
    }

    fn finish_transcribing(&mut self) {
        self.transcribing = false;
    }

    pub(super) fn provider(&self) -> Option<&str> {
        self.provider.as_deref()
    }

    pub(super) fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    fn note_space_event(&mut self) {
        if let Some(recording) = self.recording.as_mut() {
            recording.last_space_at = Instant::now();
            recording.refreshed_by_hold = true;
        }
    }

    fn should_stop_hold_recording_with_space(&self) -> bool {
        let Some(recording) = self.recording.as_ref() else {
            return false;
        };
        if recording.refreshed_by_hold {
            return false;
        }
        recording.last_space_at.elapsed() >= MANUAL_STOP_KEY_GAP
    }

    fn unavailable_once(&mut self, message: String) -> VoiceStartResult {
        if self.unavailable_reported {
            VoiceStartResult::UnavailableAlreadyReported
        } else {
            self.unavailable_reported = true;
            VoiceStartResult::Unavailable(message)
        }
    }
}

impl VoiceMode {
    fn label(self) -> &'static str {
        match self {
            Self::Hold => "hold",
            Self::Tap => "tap",
        }
    }

    pub(super) fn from_config_value(value: &str) -> Option<Self> {
        match value.trim() {
            "hold" => Some(Self::Hold),
            "tap" => Some(Self::Tap),
            _ => None,
        }
    }
}

impl VoiceCommandAction {
    fn parse(args: &str) -> Result<Self, String> {
        match args.trim() {
            "" => Ok(Self::Toggle),
            "hold" => Ok(Self::Enable(VoiceMode::Hold)),
            "tap" => Ok(Self::Enable(VoiceMode::Tap)),
            "off" | "disable" => Ok(Self::Disable),
            "status" => Ok(Self::Status),
            other => Err(format!(
                "Unknown /voice argument '{other}'. Use /voice, /voice hold, /voice tap, /voice off, or /voice status."
            )),
        }
    }
}

impl<C> TuiApp<C>
where
    C: AppClient,
{
    pub(super) async fn run_voice_slash_command(&mut self, args: &str) {
        let action = match VoiceCommandAction::parse(args) {
            Ok(action) => action,
            Err(message) => {
                self.timeline.push_error(message);
                return;
            }
        };

        match action {
            VoiceCommandAction::Toggle => {
                self.voice.enabled = !self.voice.enabled;
            }
            VoiceCommandAction::Enable(mode) => {
                self.voice.enabled = true;
                self.voice.mode = mode;
            }
            VoiceCommandAction::Disable => {
                self.voice.enabled = false;
            }
            VoiceCommandAction::Status => {}
        }

        if !matches!(action, VoiceCommandAction::Status)
            && let Err(err) = self.save_voice_config()
        {
            self.record_error(format!("failed to save voice config: {err}"));
        }

        let message = if matches!(action, VoiceCommandAction::Status) {
            self.voice.command_status()
        } else if self.voice.enabled {
            let recorder_hint = if self.voice.record_command().is_some() {
                String::new()
            } else {
                " Set [tui.voice].record_command to enable microphone capture.".to_string()
            };
            format!(
                "Voice mode enabled ({}). {} Space to record.{recorder_hint}",
                self.voice.mode.label(),
                match self.voice.mode {
                    VoiceMode::Hold => "Hold",
                    VoiceMode::Tap => "Tap",
                }
            )
        } else {
            "Voice mode disabled.".to_string()
        };
        self.timeline.push_system(message);
        self.push_event(format!("slash command: /voice {}", args.trim()));
    }

    pub(super) async fn handle_voice_key(&mut self, key: KeyEvent) -> bool {
        let composer_is_blank = composer_text(&self.composer).trim().is_empty();
        match self.voice.handle_key(key, composer_is_blank) {
            VoiceKeyAction::Pass => false,
            VoiceKeyAction::Consumed => {
                self.voice.note_space_event();
                true
            }
            VoiceKeyAction::Start => {
                if composer_is_blank {
                    self.composer = composer_textarea(self.theme);
                }
                match self.voice.start_recording() {
                    VoiceStartResult::Started => {
                        self.push_event("voice recording started".to_string());
                    }
                    VoiceStartResult::AlreadyRecording => {}
                    VoiceStartResult::Unavailable(message) => {
                        self.record_error(format!("voice recording unavailable: {message}"));
                    }
                    VoiceStartResult::UnavailableAlreadyReported => {}
                }
                true
            }
            VoiceKeyAction::Stop => {
                if let Some(recording) = self.voice.take_recording() {
                    self.finish_voice_recording(recording).await;
                }
                true
            }
        }
    }

    pub(super) async fn stop_idle_voice_recording(&mut self, now: Instant) {
        if let Some(recording) = self.voice.take_idle_hold_recording(now) {
            self.finish_voice_recording(recording).await;
        }
    }

    pub(super) async fn finish_voice_transcription_if_ready(&mut self) {
        if !self.voice.transcription_finished() {
            return;
        }
        let Some(task) = self.voice.take_transcription_task() else {
            self.voice.finish_transcribing();
            return;
        };
        self.voice.finish_transcribing();
        match task.await {
            Ok(Ok(text)) => self.insert_voice_transcript(&text),
            Ok(Err(err)) => self.record_error(format!("voice transcription failed: {err}")),
            Err(err) => self.record_error(format!("voice transcription task failed: {err}")),
        }
    }

    fn save_voice_config(&self) -> anyhow::Result<()> {
        super::save_tui_value(
            &["voice", "enabled"],
            toml::Value::Boolean(self.voice.enabled),
        )?;
        super::save_tui_value(
            &["voice", "mode"],
            toml::Value::String(self.voice.mode.label().to_string()),
        )?;
        Ok(())
    }

    pub(super) fn set_voice_model(&mut self, provider: String, model: String) {
        self.voice.provider = Some(provider.clone());
        self.voice.model = Some(model.clone());
        match self.save_voice_provider_model() {
            Ok(()) => {
                self.timeline
                    .push_system(format!("Voice model set to {provider}/{model}."));
                self.push_event(format!("voice model: {provider}/{model}"));
            }
            Err(err) => {
                self.record_error(format!("failed to save voice model: {err}"));
            }
        }
    }

    fn save_voice_provider_model(&self) -> anyhow::Result<()> {
        if let Some(provider) = self.voice.provider.as_deref() {
            super::save_tui_value(
                &["voice", "provider"],
                toml::Value::String(provider.to_string()),
            )?;
        }
        if let Some(model) = self.voice.model.as_deref() {
            super::save_tui_value(&["voice", "model"], toml::Value::String(model.to_string()))?;
        }
        Ok(())
    }

    async fn finish_voice_recording(&mut self, recording: VoiceRecording) {
        let elapsed = recording.started_at.elapsed();
        match tokio::time::timeout(RECORDING_FINISH_TIMEOUT, finish_recording(recording)).await {
            Err(_) => self.record_error("voice recording stop timed out".to_string()),
            Ok(Err(err)) => self.record_error(format!("voice recording failed: {err}")),
            Ok(Ok(audio)) if audio.is_empty() => {
                self.record_error("voice recording produced no audio".to_string());
            }
            Ok(Ok(audio)) => {
                self.push_event(format!(
                    "voice recording stopped after {:.1}s",
                    elapsed.as_secs_f32()
                ));
                self.start_voice_transcription(audio);
            }
        }
    }

    fn start_voice_transcription(&mut self, audio: Vec<u8>) {
        let client = self.client.clone();
        let provider = self.voice.provider.clone();
        let model = self.voice.model.clone();
        let mime_type = self.voice.mime_type.clone();
        let language = self.voice.language.clone();
        let task = tokio::spawn(async move {
            transcribe_voice_audio(client, provider, model, mime_type, language, audio).await
        });
        self.voice.start_transcribing(task);
        self.push_event("voice transcription started".to_string());
    }

    fn insert_voice_transcript(&mut self, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            self.push_event("voice transcript was empty".to_string());
            return;
        }
        let current = composer_text(&self.composer);
        if !current.is_empty() && !current.ends_with(char::is_whitespace) {
            self.composer.insert_str(" ");
        }
        self.composer.insert_str(text);
        self.slash_command_selection = 0;
        self.timeline.focus_composer();
        self.push_event("voice transcript inserted".to_string());
    }
}

async fn transcribe_voice_audio<C>(
    client: C,
    provider: Option<String>,
    model: Option<String>,
    mime_type: String,
    language: Option<String>,
    audio: Vec<u8>,
) -> anyhow::Result<String>
where
    C: AppClient,
{
    let res = client
        .send_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(1)),
            method: "speech/transcribe".to_string(),
            params: Some(serde_json::to_value(SpeechTranscribeParams {
                provider,
                model,
                audio: SpeechAudioPayload {
                    bytes_base64: base64::engine::general_purpose::STANDARD.encode(audio),
                    mime_type,
                    filename: Some("voice.wav".to_string()),
                },
                language,
                prompt: Some("Transcribe coding dictation for a terminal composer.".to_string()),
                diarization: false,
                metadata: BTreeMap::new(),
            })?),
        })
        .await;
    let result: SpeechTranscribeResult = decode_response(res)?;
    Ok(result.text)
}

enum VoiceStartResult {
    Started,
    AlreadyRecording,
    Unavailable(String),
    UnavailableAlreadyReported,
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn voice_duration_from_env(name: &str) -> Option<Duration> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(Duration::from_millis)
}

fn auto_record_command() -> Option<String> {
    if command_exists("rec") {
        return Some("rec -q -t wav -".to_string());
    }
    if cfg!(target_os = "macos") && command_exists("ffmpeg") {
        return Some(
            "ffmpeg -hide_banner -loglevel error -f avfoundation -i ':0' -ac 1 -ar 16000 -f wav pipe:1"
                .to_string(),
        );
    }
    if cfg!(target_os = "linux") && command_exists("arecord") {
        return Some("arecord -q -f S16_LE -r 16000 -c 1 -t wav -".to_string());
    }
    None
}

fn command_exists(command: &str) -> bool {
    let path_var = std::env::var_os("PATH").unwrap_or_else(|| OsString::from(""));
    std::env::split_paths(&path_var).any(|dir| dir.join(command).is_file())
}

fn spawn_recording_command(command: &str) -> anyhow::Result<Child> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let exec_command = format!("exec {command}");
    let child = Command::new(shell)
        .arg("-lc")
        .arg(exec_command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    Ok(child)
}

async fn finish_recording(mut recording: VoiceRecording) -> anyhow::Result<Vec<u8>> {
    drop(recording.child.stdin.take());
    let mut stdout = recording
        .child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("recording command stdout unavailable"))?;
    let mut stderr = recording
        .child
        .stderr
        .take()
        .ok_or_else(|| anyhow::anyhow!("recording command stderr unavailable"))?;
    let stdout_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).await.map(|_| bytes)
    });
    let stderr_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).await.map(|_| bytes)
    });

    request_recorder_stop(&mut recording.child).await?;
    let status = wait_for_recorder_exit(&mut recording.child).await?;
    let stdout = read_pipe_task(stdout_task).await?;
    let stderr = read_pipe_task(stderr_task).await?;
    if stdout.is_empty() && !status.success() && !stderr.is_empty() {
        anyhow::bail!("{}", String::from_utf8_lossy(&stderr).trim());
    }
    Ok(stdout)
}

async fn request_recorder_stop(child: &mut Child) -> anyhow::Result<()> {
    if child.id().is_none() {
        return Ok(());
    }
    if let Err(err) = send_recorder_signal(child, "INT").await
        && child.try_wait()?.is_none()
    {
        return Err(err);
    }
    Ok(())
}

async fn wait_for_recorder_exit(child: &mut Child) -> anyhow::Result<std::process::ExitStatus> {
    match tokio::time::timeout(RECORDER_INTERRUPT_TIMEOUT, child.wait()).await {
        Ok(status) => return Ok(status?),
        Err(_) => {
            if let Err(err) = send_recorder_signal(child, "TERM").await
                && child.try_wait()?.is_none()
            {
                return Err(err);
            }
        }
    }
    match tokio::time::timeout(RECORDER_TERMINATE_TIMEOUT, child.wait()).await {
        Ok(status) => Ok(status?),
        Err(_) => {
            child.kill().await?;
            Ok(child.wait().await?)
        }
    }
}

async fn send_recorder_signal(child: &Child, signal: &str) -> anyhow::Result<()> {
    let Some(pid) = child.id() else {
        return Ok(());
    };
    #[cfg(unix)]
    {
        let status = Command::new("kill")
            .arg(format!("-{signal}"))
            .arg(pid.to_string())
            .status()
            .await?;
        if !status.success() {
            anyhow::bail!("failed to send SIG{signal} to recorder process {pid}");
        }
    }
    #[cfg(not(unix))]
    {
        let _ = signal;
        let _ = pid;
    }
    Ok(())
}

async fn read_pipe_task(
    task: tokio::task::JoinHandle<std::io::Result<Vec<u8>>>,
) -> anyhow::Result<Vec<u8>> {
    match tokio::time::timeout(Duration::from_secs(1), task).await {
        Ok(result) => Ok(result??),
        Err(_) => Ok(Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn space(kind: KeyEventKind) -> KeyEvent {
        KeyEvent::new_with_kind(KeyCode::Char(' '), KeyModifiers::NONE, kind)
    }

    #[test]
    fn voice_command_parser_matches_claude_style_modes() {
        assert_eq!(
            VoiceCommandAction::parse("").unwrap(),
            VoiceCommandAction::Toggle
        );
        assert_eq!(
            VoiceCommandAction::parse("hold").unwrap(),
            VoiceCommandAction::Enable(VoiceMode::Hold)
        );
        assert_eq!(
            VoiceCommandAction::parse("tap").unwrap(),
            VoiceCommandAction::Enable(VoiceMode::Tap)
        );
        assert_eq!(
            VoiceCommandAction::parse("off").unwrap(),
            VoiceCommandAction::Disable
        );
        assert!(VoiceCommandAction::parse("translate").is_err());
    }

    #[tokio::test]
    async fn hold_mode_starts_on_first_blank_space_and_stops_on_release() {
        let mut state = VoiceState {
            enabled: true,
            record_command: Some("printf audio".to_string()),
            ..VoiceState::default()
        };
        assert_eq!(
            state.handle_key(space(KeyEventKind::Press), true),
            VoiceKeyAction::Start
        );
        state.recording = Some(VoiceRecording {
            child: spawn_recording_command("printf audio").unwrap(),
            started_at: Instant::now(),
            last_space_at: Instant::now(),
            refreshed_by_hold: false,
        });
        assert_eq!(
            state.handle_key(space(KeyEventKind::Press), true),
            VoiceKeyAction::Consumed
        );
        assert_eq!(
            state.handle_key(space(KeyEventKind::Repeat), true),
            VoiceKeyAction::Consumed
        );
        assert_eq!(
            state.handle_key(space(KeyEventKind::Release), true),
            VoiceKeyAction::Stop
        );
    }

    #[tokio::test]
    async fn tap_mode_toggles_when_composer_is_empty() {
        let mut state = VoiceState {
            enabled: true,
            mode: VoiceMode::Tap,
            record_command: Some("printf audio".to_string()),
            ..VoiceState::default()
        };
        assert_eq!(
            state.handle_key(space(KeyEventKind::Press), false),
            VoiceKeyAction::Pass
        );
        assert_eq!(
            state.handle_key(space(KeyEventKind::Press), true),
            VoiceKeyAction::Start
        );
        state.recording = Some(VoiceRecording {
            child: spawn_recording_command("printf audio").unwrap(),
            started_at: Instant::now(),
            last_space_at: Instant::now(),
            refreshed_by_hold: false,
        });
        assert_eq!(
            state.handle_key(space(KeyEventKind::Press), false),
            VoiceKeyAction::Stop
        );
    }

    #[tokio::test]
    async fn hold_mode_without_refresh_does_not_idle_stop() {
        let mut state = VoiceState {
            enabled: true,
            hold_idle_stop: Duration::from_millis(100),
            ..VoiceState::default()
        };
        let started_at = Instant::now() - Duration::from_secs(5);
        state.recording = Some(VoiceRecording {
            child: spawn_recording_command("printf audio").unwrap(),
            started_at,
            last_space_at: started_at,
            refreshed_by_hold: false,
        });

        assert!(state.take_idle_hold_recording(Instant::now()).is_none());
    }

    #[tokio::test]
    async fn hold_mode_idle_stops_after_refresh_gap() {
        let mut state = VoiceState {
            enabled: true,
            hold_idle_stop: Duration::from_millis(100),
            ..VoiceState::default()
        };
        let started_at = Instant::now() - Duration::from_secs(5);
        state.recording = Some(VoiceRecording {
            child: spawn_recording_command("printf audio").unwrap(),
            started_at,
            last_space_at: Instant::now() - Duration::from_millis(150),
            refreshed_by_hold: true,
        });

        assert!(state.take_idle_hold_recording(Instant::now()).is_some());
    }

    #[tokio::test]
    async fn hold_mode_second_space_after_gap_stops_recording() {
        let mut state = VoiceState {
            enabled: true,
            ..VoiceState::default()
        };
        state.recording = Some(VoiceRecording {
            child: spawn_recording_command("printf audio").unwrap(),
            started_at: Instant::now() - Duration::from_secs(2),
            last_space_at: Instant::now() - MANUAL_STOP_KEY_GAP - Duration::from_millis(1),
            refreshed_by_hold: false,
        });

        assert_eq!(
            state.handle_key(space(KeyEventKind::Press), true),
            VoiceKeyAction::Stop
        );
    }

    #[tokio::test]
    async fn hold_mode_refresh_prevents_space_from_toggling_stop() {
        let mut state = VoiceState {
            enabled: true,
            ..VoiceState::default()
        };
        state.recording = Some(VoiceRecording {
            child: spawn_recording_command("printf audio").unwrap(),
            started_at: Instant::now() - Duration::from_secs(2),
            last_space_at: Instant::now() - MANUAL_STOP_KEY_GAP - Duration::from_millis(1),
            refreshed_by_hold: true,
        });

        assert_eq!(
            state.handle_key(space(KeyEventKind::Press), true),
            VoiceKeyAction::Consumed
        );
    }
}
