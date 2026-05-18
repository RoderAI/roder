use std::path::PathBuf;

use roder_api::{
    interactive::{
        HandlerOutcome, InteractiveEvent, InteractiveMouseButton, InteractiveRegion, RegionKind,
    },
    policy_mode::{PolicyMode, PolicyModeConfig},
};
use serde::{Deserialize, Serialize};
use tokio::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranscriptActionResult {
    OpenedUrl { url: String },
    OpenedFile { path: PathBuf, line: Option<u32> },
    EmittedOpenFileEvent { path: PathBuf, line: Option<u32> },
    CopiedToClipboard { text: String, reason: String },
}

#[async_trait::async_trait]
pub trait ScopedProcessRunner: Send + Sync + 'static {
    async fn run_scoped(&self, program: &str, args: &[String]) -> anyhow::Result<()>;
}

#[async_trait::async_trait]
pub trait ClipboardSink: Send + Sync + 'static {
    async fn write_text(&self, text: &str) -> anyhow::Result<()>;
}

#[async_trait::async_trait]
pub trait TranscriptAppEventSink: Send + Sync + 'static {
    async fn emit_open_file(&self, event: TranscriptOpenFileEvent) -> anyhow::Result<()>;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TranscriptOpenFileEvent {
    pub path: PathBuf,
    pub line: Option<u32>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemProcessRunner;

#[async_trait::async_trait]
impl ScopedProcessRunner for SystemProcessRunner {
    async fn run_scoped(&self, program: &str, args: &[String]) -> anyhow::Result<()> {
        let status = Command::new(program).args(args).status().await?;
        if !status.success() {
            anyhow::bail!("{program} exited with status {status}");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClipboardSink;

#[async_trait::async_trait]
impl ClipboardSink for SystemClipboardSink {
    async fn write_text(&self, text: &str) -> anyhow::Result<()> {
        write_to_system_clipboard(text).await
    }
}

#[derive(Debug, Clone)]
pub struct TranscriptRegionActions<R, C> {
    process_runner: R,
    clipboard: C,
    policy_mode: PolicyMode,
}

#[derive(Debug, Clone)]
pub struct TranscriptFileActions<R, E> {
    process_runner: R,
    event_sink: E,
    policy_mode: PolicyMode,
    headless: bool,
    editor: Option<String>,
}

impl<R, C> TranscriptRegionActions<R, C>
where
    R: ScopedProcessRunner,
    C: ClipboardSink,
{
    pub fn new(process_runner: R, clipboard: C, policy_mode: PolicyMode) -> Self {
        Self {
            process_runner,
            clipboard,
            policy_mode,
        }
    }

    pub async fn open_url(&self, url: &str) -> anyhow::Result<TranscriptActionResult> {
        if PolicyModeConfig::for_mode(self.policy_mode).allow_process {
            let opener = UrlOpenCommand::for_platform(url);
            match self
                .process_runner
                .run_scoped(&opener.program, &opener.args)
                .await
            {
                Ok(()) => {
                    return Ok(TranscriptActionResult::OpenedUrl {
                        url: url.to_string(),
                    });
                }
                Err(err) => {
                    self.clipboard.write_text(url).await?;
                    return Ok(TranscriptActionResult::CopiedToClipboard {
                        text: url.to_string(),
                        reason: format!("open URL failed: {err}"),
                    });
                }
            }
        }

        self.clipboard.write_text(url).await?;
        Ok(TranscriptActionResult::CopiedToClipboard {
            text: url.to_string(),
            reason: format!(
                "process launch blocked by active policy mode {:?}",
                self.policy_mode
            ),
        })
    }
}

impl<R, E> TranscriptFileActions<R, E>
where
    R: ScopedProcessRunner,
    E: TranscriptAppEventSink,
{
    pub fn new(
        process_runner: R,
        event_sink: E,
        policy_mode: PolicyMode,
        headless: bool,
        editor: Option<String>,
    ) -> Self {
        Self {
            process_runner,
            event_sink,
            policy_mode,
            headless,
            editor,
        }
    }

    pub async fn open_file_reference(
        &self,
        path: PathBuf,
        line: Option<u32>,
    ) -> anyhow::Result<TranscriptActionResult> {
        if !self.headless
            && PolicyModeConfig::for_mode(self.policy_mode).allow_process
            && let Some(command) = self.editor_command(&path, line)
        {
            self.process_runner
                .run_scoped(&command.program, &command.args)
                .await?;
            return Ok(TranscriptActionResult::OpenedFile { path, line });
        }

        self.event_sink
            .emit_open_file(TranscriptOpenFileEvent {
                path: path.clone(),
                line,
            })
            .await?;
        Ok(TranscriptActionResult::EmittedOpenFileEvent { path, line })
    }

    fn editor_command(&self, path: &std::path::Path, line: Option<u32>) -> Option<EditorCommand> {
        let editor = self.editor.as_deref()?.trim();
        if editor.is_empty() {
            return None;
        }

        let mut parts = editor
            .split_whitespace()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let program = parts.first().cloned()?;
        let mut args = parts.split_off(1);
        let path_arg = match line {
            Some(line) if editor_supports_plus_line(&program) => {
                args.push(format!("+{line}"));
                path.to_string_lossy().to_string()
            }
            _ => path.to_string_lossy().to_string(),
        };
        args.push(path_arg);
        Some(EditorCommand { program, args })
    }
}

#[async_trait::async_trait]
impl<R, C> roder_api::interactive::InteractiveRegionHandler for TranscriptRegionActions<R, C>
where
    R: ScopedProcessRunner,
    C: ClipboardSink,
{
    fn id(&self) -> String {
        "transcript-actions".to_string()
    }

    fn kinds(&self) -> &[&'static str] {
        &["Url"]
    }

    async fn handle(
        &self,
        event: InteractiveEvent,
        region: &InteractiveRegion,
    ) -> anyhow::Result<HandlerOutcome> {
        let InteractiveEvent::Click { button, .. } = event else {
            return Ok(HandlerOutcome::Passthrough);
        };
        if button != InteractiveMouseButton::Left {
            return Ok(HandlerOutcome::Passthrough);
        }
        let RegionKind::Url(url) = &region.kind else {
            return Ok(HandlerOutcome::Passthrough);
        };

        self.open_url(url).await?;
        Ok(HandlerOutcome::Consumed)
    }
}

#[async_trait::async_trait]
impl<R, E> roder_api::interactive::InteractiveRegionHandler for TranscriptFileActions<R, E>
where
    R: ScopedProcessRunner,
    E: TranscriptAppEventSink,
{
    fn id(&self) -> String {
        "transcript-file-actions".to_string()
    }

    fn kinds(&self) -> &[&'static str] {
        &["FileReference"]
    }

    async fn handle(
        &self,
        event: InteractiveEvent,
        region: &InteractiveRegion,
    ) -> anyhow::Result<HandlerOutcome> {
        let InteractiveEvent::Click { button, .. } = event else {
            return Ok(HandlerOutcome::Passthrough);
        };
        if button != InteractiveMouseButton::Left {
            return Ok(HandlerOutcome::Passthrough);
        }
        let RegionKind::FileReference { path, line } = &region.kind else {
            return Ok(HandlerOutcome::Passthrough);
        };

        self.open_file_reference(path.clone(), *line).await?;
        Ok(HandlerOutcome::Consumed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UrlOpenCommand {
    program: String,
    args: Vec<String>,
}

impl UrlOpenCommand {
    fn for_platform(url: &str) -> Self {
        #[cfg(target_os = "macos")]
        {
            Self {
                program: "open".to_string(),
                args: vec![url.to_string()],
            }
        }
        #[cfg(target_os = "windows")]
        {
            Self {
                program: "cmd".to_string(),
                args: vec![
                    "/C".to_string(),
                    "start".to_string(),
                    "".to_string(),
                    url.to_string(),
                ],
            }
        }
        #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
        {
            Self {
                program: "xdg-open".to_string(),
                args: vec![url.to_string()],
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EditorCommand {
    program: String,
    args: Vec<String>,
}

fn editor_supports_plus_line(program: &str) -> bool {
    let name = std::path::Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(program);
    matches!(name, "vi" | "vim" | "nvim" | "nano" | "emacs")
}

async fn write_to_system_clipboard(text: &str) -> anyhow::Result<()> {
    let candidates: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("pbcopy", &[])]
    } else if cfg!(target_os = "windows") {
        &[("clip", &[])]
    } else {
        &[
            ("wl-copy", &[]),
            ("xclip", &["-selection", "clipboard"]),
            ("xsel", &["--clipboard", "--input"]),
        ]
    };

    for (program, args) in candidates {
        if write_with_clipboard_command(program, args, text)
            .await
            .is_ok()
        {
            return Ok(());
        }
    }

    anyhow::bail!("no clipboard command available")
}

async fn write_with_clipboard_command(
    program: &str,
    args: &[&str],
    text: &str,
) -> anyhow::Result<()> {
    use tokio::io::AsyncWriteExt;

    let mut child = Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(text.as_bytes()).await?;
    }

    let status = child.wait().await?;
    if !status.success() {
        anyhow::bail!("{program} exited with status {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use roder_api::interactive::{
        HoverCursor, InteractiveModifiers, InteractiveRegionHandler, KeyChord, RegionId, RegionRect,
    };

    use super::*;

    type CommandCalls = Arc<Mutex<Vec<(String, Vec<String>)>>>;

    #[derive(Clone, Default)]
    struct FakeRunner {
        calls: CommandCalls,
        fail: bool,
    }

    #[async_trait::async_trait]
    impl ScopedProcessRunner for FakeRunner {
        async fn run_scoped(&self, program: &str, args: &[String]) -> anyhow::Result<()> {
            self.calls
                .lock()
                .unwrap()
                .push((program.to_string(), args.to_vec()));
            if self.fail {
                anyhow::bail!("runner failed");
            }
            Ok(())
        }
    }

    #[derive(Clone, Default)]
    struct FakeClipboard {
        writes: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait::async_trait]
    impl ClipboardSink for FakeClipboard {
        async fn write_text(&self, text: &str) -> anyhow::Result<()> {
            self.writes.lock().unwrap().push(text.to_string());
            Ok(())
        }
    }

    #[derive(Clone, Default)]
    struct FakeEvents {
        open_file_events: Arc<Mutex<Vec<TranscriptOpenFileEvent>>>,
    }

    #[async_trait::async_trait]
    impl TranscriptAppEventSink for FakeEvents {
        async fn emit_open_file(&self, event: TranscriptOpenFileEvent) -> anyhow::Result<()> {
            self.open_file_events.lock().unwrap().push(event);
            Ok(())
        }
    }

    #[tokio::test]
    async fn open_url_uses_platform_opener_when_policy_allows_processes() {
        let runner = FakeRunner::default();
        let clipboard = FakeClipboard::default();
        let actions =
            TranscriptRegionActions::new(runner.clone(), clipboard.clone(), PolicyMode::Default);

        let result = actions.open_url("https://example.com").await.unwrap();

        assert_eq!(
            result,
            TranscriptActionResult::OpenedUrl {
                url: "https://example.com".to_string()
            }
        );
        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1.last().unwrap(), "https://example.com");
        assert!(clipboard.writes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn open_url_copies_to_clipboard_when_policy_blocks_processes() {
        let runner = FakeRunner::default();
        let clipboard = FakeClipboard::default();
        let actions =
            TranscriptRegionActions::new(runner.clone(), clipboard.clone(), PolicyMode::Plan);

        let result = actions.open_url("https://example.com").await.unwrap();

        assert!(matches!(
            result,
            TranscriptActionResult::CopiedToClipboard { .. }
        ));
        assert!(runner.calls.lock().unwrap().is_empty());
        assert_eq!(
            clipboard.writes.lock().unwrap().as_slice(),
            &["https://example.com".to_string()]
        );
    }

    #[tokio::test]
    async fn open_url_falls_back_to_clipboard_when_opener_fails() {
        let runner = FakeRunner {
            fail: true,
            ..FakeRunner::default()
        };
        let clipboard = FakeClipboard::default();
        let actions =
            TranscriptRegionActions::new(runner.clone(), clipboard.clone(), PolicyMode::Default);

        let result = actions.open_url("https://example.com").await.unwrap();

        assert!(matches!(
            result,
            TranscriptActionResult::CopiedToClipboard { .. }
        ));
        assert_eq!(runner.calls.lock().unwrap().len(), 1);
        assert_eq!(
            clipboard.writes.lock().unwrap().as_slice(),
            &["https://example.com".to_string()]
        );
    }

    #[tokio::test]
    async fn url_click_handler_consumes_left_clicks() {
        let actions = TranscriptRegionActions::new(
            FakeRunner::default(),
            FakeClipboard::default(),
            PolicyMode::Default,
        );
        let region = InteractiveRegion {
            id: RegionId::from("url-1"),
            rect: RegionRect {
                x: 0,
                y: 0,
                width: 10,
                height: 1,
            },
            z: 0,
            kind: RegionKind::Url("https://example.com".to_string()),
            hover_cursor: HoverCursor::Pointer,
            keyboard_binding: Some(KeyChord {
                key: "enter".to_string(),
                modifiers: InteractiveModifiers::default(),
            }),
        };

        let outcome = actions
            .handle(
                InteractiveEvent::Click {
                    region: "url-1".to_string(),
                    modifiers: InteractiveModifiers::default(),
                    button: InteractiveMouseButton::Left,
                },
                &region,
            )
            .await
            .unwrap();

        assert_eq!(outcome, HandlerOutcome::Consumed);
    }

    #[tokio::test]
    async fn open_file_reference_uses_editor_when_policy_allows_processes() {
        let runner = FakeRunner::default();
        let events = FakeEvents::default();
        let actions = TranscriptFileActions::new(
            runner.clone(),
            events.clone(),
            PolicyMode::Default,
            false,
            Some("nvim".to_string()),
        );

        let result = actions
            .open_file_reference(PathBuf::from("src/lib.rs"), Some(42))
            .await
            .unwrap();

        assert_eq!(
            result,
            TranscriptActionResult::OpenedFile {
                path: PathBuf::from("src/lib.rs"),
                line: Some(42)
            }
        );
        assert_eq!(
            runner.calls.lock().unwrap().as_slice(),
            &[(
                "nvim".to_string(),
                vec!["+42".to_string(), "src/lib.rs".to_string()]
            )]
        );
        assert!(events.open_file_events.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn open_file_reference_emits_app_server_event_when_headless() {
        let runner = FakeRunner::default();
        let events = FakeEvents::default();
        let actions = TranscriptFileActions::new(
            runner.clone(),
            events.clone(),
            PolicyMode::Default,
            true,
            Some("nvim".to_string()),
        );

        let result = actions
            .open_file_reference(PathBuf::from("src/lib.rs"), Some(42))
            .await
            .unwrap();

        assert_eq!(
            result,
            TranscriptActionResult::EmittedOpenFileEvent {
                path: PathBuf::from("src/lib.rs"),
                line: Some(42)
            }
        );
        assert!(runner.calls.lock().unwrap().is_empty());
        assert_eq!(
            events.open_file_events.lock().unwrap().as_slice(),
            &[TranscriptOpenFileEvent {
                path: PathBuf::from("src/lib.rs"),
                line: Some(42)
            }]
        );
    }

    #[tokio::test]
    async fn open_file_reference_emits_app_server_event_when_policy_blocks_processes() {
        let runner = FakeRunner::default();
        let events = FakeEvents::default();
        let actions = TranscriptFileActions::new(
            runner.clone(),
            events.clone(),
            PolicyMode::Plan,
            false,
            Some("nvim".to_string()),
        );

        let result = actions
            .open_file_reference(PathBuf::from("src/lib.rs"), None)
            .await
            .unwrap();

        assert!(matches!(
            result,
            TranscriptActionResult::EmittedOpenFileEvent { .. }
        ));
        assert!(runner.calls.lock().unwrap().is_empty());
        assert_eq!(
            events.open_file_events.lock().unwrap().as_slice(),
            &[TranscriptOpenFileEvent {
                path: PathBuf::from("src/lib.rs"),
                line: None
            }]
        );
    }

    #[tokio::test]
    async fn file_reference_click_handler_consumes_left_clicks() {
        let actions = TranscriptFileActions::new(
            FakeRunner::default(),
            FakeEvents::default(),
            PolicyMode::Plan,
            false,
            Some("nvim".to_string()),
        );
        let region = InteractiveRegion {
            id: RegionId::from("file-1"),
            rect: RegionRect {
                x: 0,
                y: 0,
                width: 10,
                height: 1,
            },
            z: 0,
            kind: RegionKind::FileReference {
                path: PathBuf::from("src/lib.rs"),
                line: Some(42),
            },
            hover_cursor: HoverCursor::Pointer,
            keyboard_binding: Some(KeyChord {
                key: "enter".to_string(),
                modifiers: InteractiveModifiers::default(),
            }),
        };

        let outcome = actions
            .handle(
                InteractiveEvent::Click {
                    region: "file-1".to_string(),
                    modifiers: InteractiveModifiers::default(),
                    button: InteractiveMouseButton::Left,
                },
                &region,
            )
            .await
            .unwrap();

        assert_eq!(outcome, HandlerOutcome::Consumed);
    }
}
