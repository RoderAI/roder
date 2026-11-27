//! Consent-driven startup setup, before a provider-backed TUI can initialize.
mod render;
#[cfg(test)]
mod tests;

use std::{io, sync::Arc};

use anyhow::{Context, Result};
use async_trait::async_trait;
use crossterm::{
    cursor::MoveTo,
    event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{
        Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
        enable_raw_mode,
    },
};
use futures::StreamExt;
use ratatui::{Terminal, TerminalOptions, Viewport, backend::CrosstermBackend, layout::Rect};
use tokio::{sync::mpsc, task::JoinHandle};

use super::Theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupStatus {
    MissingRuntime,
    NeedsSignIn,
    Ready,
}

#[derive(Clone, Debug)]
pub struct SetupCheck {
    pub status: SetupStatus,
    /// User-facing, non-secret details such as the runtime installation path.
    pub detail: String,
}

/// Provider operations are kept outside the renderer for testing and reuse.
#[async_trait]
pub trait ProviderSetupDriver: Send + Sync {
    async fn check(&self) -> Result<SetupCheck>;
    /// Called only after explicit installation consent. Must be cancellation-safe.
    /// Progress messages must not contain authentication codes or credentials.
    async fn install(&self, progress: mpsc::UnboundedSender<String>) -> Result<()>;
    /// Runs with a cooked, inherited terminal. Do not capture login output.
    async fn sign_in(&self) -> Result<()>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Install,
    SignIn,
    Check,
    Continue,
    Cancel,
}

struct Model {
    check: SetupCheck,
    busy: Option<Action>,
    error: Option<String>,
    retry: Option<Action>,
    progress: String,
    scroll: u16,
}

impl Model {
    fn new(check: SetupCheck) -> Self {
        Self {
            check,
            busy: None,
            error: None,
            retry: None,
            progress: String::new(),
            scroll: 0,
        }
    }

    fn key(&mut self, key: KeyEvent) -> Option<Action> {
        if key.kind != KeyEventKind::Press {
            return None;
        }
        if key.code == KeyCode::Esc
            || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
        {
            return Some(Action::Cancel);
        }
        if self.busy.is_none() && matches!(key.code, KeyCode::Char('r' | 'R')) {
            return Some(Action::Check);
        }
        match key.code {
            KeyCode::Down | KeyCode::PageDown => {
                self.scroll = self.scroll.saturating_add(1);
                return None;
            }
            KeyCode::Up | KeyCode::PageUp => {
                self.scroll = self.scroll.saturating_sub(1);
                return None;
            }
            _ => {}
        }
        if key.code != KeyCode::Enter || self.busy.is_some() {
            return None;
        }
        Some(self.retry.unwrap_or(match self.check.status {
            SetupStatus::MissingRuntime => Action::Install,
            SetupStatus::NeedsSignIn => Action::SignIn,
            SetupStatus::Ready => Action::Continue,
        }))
    }

    fn start(&mut self, action: Action) {
        self.busy = Some(action);
        self.error = None;
        self.retry = None;
        self.progress.clear();
        self.scroll = 0;
    }

    fn failed(&mut self, action: Action, error: anyhow::Error) {
        self.busy = None;
        self.error = Some(format!("{error:#}"));
        self.retry = Some(action);
        self.progress.clear();
        self.scroll = 0;
    }

    fn checked(&mut self, check: SetupCheck) {
        self.check = check;
        self.busy = None;
        self.error = None;
        self.retry = None;
        self.progress.clear();
        self.scroll = 0;
    }
}

enum Completion {
    Installed,
    Checked(SetupCheck),
}

/// Dropping a setup run also cancels its download/check future.
#[derive(Default)]
struct Worker(Option<JoinHandle<Result<Completion>>>);

impl Worker {
    fn start(
        &mut self,
        action: Action,
        driver: Arc<dyn ProviderSetupDriver>,
        progress: mpsc::UnboundedSender<String>,
    ) {
        self.0 = Some(tokio::spawn(async move {
            match action {
                Action::Install => {
                    driver.install(progress).await?;
                    Ok(Completion::Installed)
                }
                Action::Check => Ok(Completion::Checked(driver.check().await?)),
                _ => unreachable!("only background actions start a worker"),
            }
        }));
    }

    async fn finish(&mut self) -> Result<Completion> {
        match &mut self.0 {
            Some(task) => task.await.context("Provider setup task failed")?,
            None => std::future::pending().await,
        }
    }

    async fn cancel(&mut self) {
        if let Some(task) = self.0.take() {
            task.abort();
            let _ = task.await;
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        if let Some(task) = &self.0 {
            task.abort();
        }
    }
}

struct SetupTerminal {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
    active: bool,
}

impl SetupTerminal {
    fn enter() -> Result<Self> {
        enable_raw_mode()?;
        let terminal = match Self::new_terminal() {
            Ok(terminal) => terminal,
            Err(error) => {
                let _ = disable_raw_mode();
                return Err(error.into());
            }
        };
        let mut session = Self {
            terminal,
            active: true,
        };
        session.resume()?;
        Ok(session)
    }

    fn resume(&mut self) -> Result<()> {
        enable_raw_mode()?;
        self.active = true;
        execute!(self.terminal.backend_mut(), EnterAlternateScreen)?;
        self.refresh()?;
        Ok(())
    }

    fn new_terminal() -> io::Result<Terminal<CrosstermBackend<io::Stdout>>> {
        let (width, height) = crossterm::terminal::size()?;
        // A fixed viewport is refreshed on Resize events. Ratatui's automatic
        // clear/resize queries cursor position, which conflicts with EventStream.
        Terminal::with_options(
            CrosstermBackend::new(io::stdout()),
            TerminalOptions {
                viewport: Viewport::Fixed(Rect::new(0, 0, width, height)),
            },
        )
    }

    fn refresh(&mut self) -> Result<()> {
        execute!(
            self.terminal.backend_mut(),
            Clear(ClearType::All),
            MoveTo(0, 0)
        )?;
        self.terminal = Self::new_terminal()?;
        self.terminal.hide_cursor()?;
        Ok(())
    }

    fn restore(&mut self) -> Result<()> {
        if self.active {
            // Attempt both even when restoring one terminal feature fails.
            let raw = disable_raw_mode();
            let screen = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
            let cursor = self.terminal.show_cursor();
            raw?;
            screen?;
            cursor?;
            self.active = false;
        }
        Ok(())
    }
}

impl Drop for SetupTerminal {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

/// Guide installation and Google sign-in. Returns `false` on explicit cancel.
/// Already-ready providers skip the wizard without touching the terminal.
pub async fn run_provider_setup(
    driver: Arc<dyn ProviderSetupDriver>,
    initial: SetupCheck,
) -> Result<bool> {
    if initial.status == SetupStatus::Ready {
        return Ok(true);
    }
    let mut terminal = SetupTerminal::enter()?;
    let theme = Theme::for_terminal_themed();
    let mut model = Model::new(initial);
    let mut reader = Some(EventStream::new());
    let mut worker = Worker::default();
    let (progress_tx, mut progress_rx) = mpsc::unbounded_channel();

    loop {
        terminal
            .terminal
            .draw(|frame| render::draw(frame, &model, theme))?;
        tokio::select! {
            event = reader.as_mut().expect("reader exists outside sign-in").next() => {
                let event = event.context("Provider setup input closed")??;
                if matches!(event, Event::Resize(..)) {
                    terminal.refresh()?;
                    continue;
                }
                let Event::Key(key) = event else { continue };
                let Some(action) = model.key(key) else { continue };
                match action {
                    Action::Cancel => {
                        worker.cancel().await;
                        terminal.restore()?;
                        return Ok(false);
                    }
                    Action::Continue => {
                        terminal.restore()?;
                        return Ok(true);
                    }
                    Action::Install | Action::Check => {
                        model.start(action);
                        worker.start(action, driver.clone(), progress_tx.clone());
                    }
                    Action::SignIn => {
                        model.start(action);
                        // No EventStream may compete with the official login for stdin.
                        drop(reader.take());
                        terminal.restore()?;
                        println!("\nGoogle Antigravity sign-in\n");
                        println!("Follow the official Google URL/code instructions below.");
                        println!("Use the Google account with your Antigravity subscription.");
                        println!("Return here after signing in. Ctrl+C cancels the login.\n");
                        let result = driver.sign_in().await;
                        terminal.resume()?;
                        reader = Some(EventStream::new());
                        match result {
                            Ok(()) => {
                                model.start(Action::Check);
                                worker.start(Action::Check, driver.clone(), progress_tx.clone());
                            }
                            Err(error) => model.failed(Action::SignIn, error),
                        }
                    }
                }
            }
            Some(progress) = progress_rx.recv() => {
                if model.busy == Some(Action::Install) {
                    model.progress = progress;
                }
            }
            completion = worker.finish() => {
                worker.0.take();
                match completion {
                    Ok(Completion::Installed) => {
                        model.start(Action::Check);
                        worker.start(Action::Check, driver.clone(), progress_tx.clone());
                    }
                    Ok(Completion::Checked(check)) => model.checked(check),
                    Err(error) => model.failed(model.busy.unwrap_or(Action::Check), error),
                }
            }
        }
    }
}
