use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use ratatui::{backend::TestBackend, style::Color};
use tokio::sync::Notify;

use super::*;

fn check(status: SetupStatus) -> SetupCheck {
    SetupCheck {
        status,
        detail: "Runtime location: user directory".into(),
    }
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[derive(Default)]
struct FakeDriver {
    installs: AtomicUsize,
    sign_ins: AtomicUsize,
    started: Notify,
    cancelled: Arc<AtomicBool>,
}

struct CancelGuard(Arc<AtomicBool>);

impl Drop for CancelGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[async_trait]
impl ProviderSetupDriver for FakeDriver {
    async fn check(&self) -> Result<SetupCheck> {
        Ok(check(SetupStatus::Ready))
    }

    async fn install(&self, progress: mpsc::UnboundedSender<String>) -> Result<()> {
        self.installs.fetch_add(1, Ordering::SeqCst);
        let _guard = CancelGuard(self.cancelled.clone());
        let _ = progress.send("Downloading runtime".into());
        self.started.notify_one();
        std::future::pending().await
    }

    async fn sign_in(&self) -> Result<()> {
        self.sign_ins.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
async fn missing_runtime_never_installs_without_consent() {
    let driver = Arc::new(FakeDriver::default());
    let mut model = Model::new(check(SetupStatus::MissingRuntime));
    assert_eq!(model.busy, None);
    assert_eq!(model.key(key(KeyCode::Char('y'))), None);
    assert_eq!(model.key(key(KeyCode::Down)), None);
    assert_eq!(driver.installs.load(Ordering::SeqCst), 0);
    assert_eq!(model.key(key(KeyCode::Enter)), Some(Action::Install));
    // The action is just intent: only the event loop may start the operation.
    assert_eq!(driver.installs.load(Ordering::SeqCst), 0);
}

#[test]
fn cancel_works_in_every_setup_state() {
    for status in [
        SetupStatus::MissingRuntime,
        SetupStatus::NeedsSignIn,
        SetupStatus::Ready,
    ] {
        let mut model = Model::new(check(status));
        assert_eq!(model.key(key(KeyCode::Esc)), Some(Action::Cancel));
        model.start(Action::Install);
        assert_eq!(model.key(key(KeyCode::Esc)), Some(Action::Cancel));
        assert_eq!(
            model.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Some(Action::Cancel)
        );
    }
}

#[test]
fn busy_ignores_enter_and_key_repeat() {
    let mut model = Model::new(check(SetupStatus::MissingRuntime));
    model.start(Action::Install);
    assert_eq!(model.key(key(KeyCode::Enter)), None);
    model.checked(check(SetupStatus::NeedsSignIn));
    let mut repeat = key(KeyCode::Enter);
    repeat.kind = KeyEventKind::Repeat;
    assert_eq!(model.key(repeat), None);
    assert_eq!(model.key(key(KeyCode::Enter)), Some(Action::SignIn));
}

#[test]
fn recheck_is_available_when_idle_but_not_during_an_operation() {
    for status in [
        SetupStatus::MissingRuntime,
        SetupStatus::NeedsSignIn,
        SetupStatus::Ready,
    ] {
        let mut model = Model::new(check(status));
        assert_eq!(model.key(key(KeyCode::Char('r'))), Some(Action::Check));
        assert_eq!(model.key(key(KeyCode::Char('R'))), Some(Action::Check));
        model.failed(Action::SignIn, anyhow::anyhow!("Try again"));
        assert_eq!(model.key(key(KeyCode::Char('r'))), Some(Action::Check));
        model.start(Action::Install);
        assert_eq!(model.key(key(KeyCode::Char('r'))), None);
        assert_eq!(model.key(key(KeyCode::Char('R'))), None);
    }
}

#[test]
fn failures_retry_the_failed_operation_and_clear_after_success() {
    let mut model = Model::new(check(SetupStatus::MissingRuntime));
    for action in [Action::Install, Action::SignIn, Action::Check] {
        model.start(action);
        model.failed(action, anyhow::anyhow!("Connection unavailable"));
        assert!(model.error.as_deref().unwrap().contains("Connection"));
        assert_eq!(model.key(key(KeyCode::Enter)), Some(action));
        model.start(action);
        assert!(model.error.is_none());
        assert_eq!(model.key(key(KeyCode::Enter)), None);
    }
    model.checked(check(SetupStatus::Ready));
    assert_eq!(model.key(key(KeyCode::Enter)), Some(Action::Continue));
    assert!(model.retry.is_none());
}

#[tokio::test]
async fn cancelling_busy_install_drops_the_driver_future() {
    let driver = Arc::new(FakeDriver::default());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut worker = Worker::default();
    worker.start(Action::Install, driver.clone(), tx);
    driver.started.notified().await;
    assert_eq!(rx.recv().await.as_deref(), Some("Downloading runtime"));
    assert_eq!(driver.installs.load(Ordering::SeqCst), 1);
    worker.cancel().await;
    assert!(driver.cancelled.load(Ordering::SeqCst));
    assert!(worker.0.is_none());
}

#[tokio::test]
async fn dropping_worker_also_cancels_owned_install() {
    let driver = Arc::new(FakeDriver::default());
    let (tx, _) = mpsc::unbounded_channel();
    let mut worker = Worker::default();
    worker.start(Action::Install, driver.clone(), tx);
    driver.started.notified().await;
    drop(worker);
    tokio::task::yield_now().await;
    assert!(driver.cancelled.load(Ordering::SeqCst));
}

#[tokio::test]
async fn already_ready_skips_terminal_and_provider_operations() {
    let driver = Arc::new(FakeDriver::default());
    assert!(
        run_provider_setup(driver.clone(), check(SetupStatus::Ready))
            .await
            .unwrap()
    );
    assert_eq!(driver.installs.load(Ordering::SeqCst), 0);
    assert_eq!(driver.sign_ins.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn check_completion_transitions_to_ready_without_installing() {
    let driver = Arc::new(FakeDriver::default());
    let (tx, _) = mpsc::unbounded_channel();
    let mut worker = Worker::default();
    worker.start(Action::Check, driver.clone(), tx);
    let Completion::Checked(result) = worker.finish().await.unwrap() else {
        panic!("expected checked completion");
    };
    let mut model = Model::new(check(SetupStatus::NeedsSignIn));
    model.checked(result);
    assert_eq!(model.key(key(KeyCode::Enter)), Some(Action::Continue));
    assert_eq!(driver.installs.load(Ordering::SeqCst), 0);
}

#[test]
fn small_terminal_keeps_consent_and_cancel_visible_with_semantic_theme() {
    for dark in [true, false] {
        let mut theme =
            Theme::for_terminal_capabilities(dark, super::super::TerminalColorDepth::TrueColor);
        // Prove the renderer consumes semantic dialog styles, not inline colors.
        theme.accent = Color::Indexed(105);
        let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
        let model = Model::new(check(SetupStatus::MissingRuntime));
        terminal
            .draw(|frame| render::draw(frame, &model, theme))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text: String = buffer.content().iter().map(|cell| cell.symbol()).collect();
        assert!(text.contains("Google Antigravity setup"));
        assert!(text.contains("Enter: install"));
        assert!(text.contains("Esc / Ctrl+C cancel"));
        assert!(buffer.content().iter().any(|cell| cell.fg == theme.accent));
        assert!(
            buffer.content().iter().all(|cell| {
                cell.bg == theme.dialog_bg || Some(cell.bg) == theme.dialog_key().bg
            })
        );
    }
}

#[test]
fn tiny_terminals_and_all_states_render_without_panicking() {
    for (width, height) in [(1, 1), (20, 4), (80, 24)] {
        for status in [
            SetupStatus::MissingRuntime,
            SetupStatus::NeedsSignIn,
            SetupStatus::Ready,
        ] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let mut model = Model::new(check(status));
            model.failed(Action::Check, anyhow::anyhow!("Test error"));
            terminal
                .draw(|frame| render::draw(frame, &model, Theme::for_terminal()))
                .unwrap();
        }
    }
}

#[test]
fn busy_progress_and_retry_error_are_visible_without_scrolling() {
    let mut model = Model::new(check(SetupStatus::MissingRuntime));
    model.start(Action::Install);
    model.progress = "Downloaded 12 MiB".into();
    let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
    terminal
        .draw(|frame| render::draw(frame, &model, Theme::for_terminal()))
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("Downloaded 12 MiB"));
    model.failed(Action::Install, anyhow::anyhow!("Network unavailable"));
    terminal
        .draw(|frame| render::draw(frame, &model, Theme::for_terminal()))
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("Network unavailable"));
    assert!(text.contains("Enter: retry installation"));
}
