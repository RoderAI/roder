use std::io;
use std::time::Duration;

use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
};
use roder_api::events::RoderEvent;
use roder_app_server::LocalAppClient;
use roder_protocol::{CreateSessionResult, InterruptTurnParams, JsonRpcRequest, StartTurnParams};

struct Theme;

impl Theme {
    fn header() -> Style {
        Style::default().bg(Color::Blue).fg(Color::Reset)
    }
    fn footer() -> Style {
        Style::default().bg(Color::Blue).fg(Color::Reset)
    }
    fn highlight() -> Style {
        Style::default().bg(Color::Blue).fg(Color::Reset)
    }
}

pub struct TuiApp {
    client: LocalAppClient,
    thread_id: String,
    active_turn_id: Option<String>,
    provider: String,
    model: String,
    input: String,
    messages: Vec<String>,
    events: Vec<String>,
    show_ctrl_p_menu: bool,
    menu_state: ListState,
}

impl TuiApp {
    pub async fn new(client: LocalAppClient, model: String) -> anyhow::Result<Self> {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(1)),
            method: "sessions/create".to_string(),
            params: None,
        };

        let res = client.send_request(req).await;
        let session = if let Some(result) = res.result {
            serde_json::from_value::<CreateSessionResult>(result)?
        } else {
            anyhow::bail!("failed to create session: {:?}", res.error);
        };

        let mut menu_state = ListState::default();
        menu_state.select(Some(0));

        Ok(Self {
            client,
            thread_id: session.thread_id,
            active_turn_id: None,
            provider: session.provider,
            model: if model.is_empty() {
                session.model
            } else {
                model
            },
            input: String::new(),
            messages: Vec::new(),
            events: Vec::new(),
            show_ctrl_p_menu: false,
            menu_state,
        })
    }

    pub async fn run(&mut self) -> anyhow::Result<()> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let mut rx = self.client.subscribe_events();

        loop {
            terminal.draw(|f| {
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(1),
                        Constraint::Min(5),
                        Constraint::Length(3),
                        Constraint::Length(1),
                    ])
                    .split(f.size());

                let header = Paragraph::new(format!(
                    " Roder  provider: {}/{}  session: {}  turn: {}",
                    self.provider,
                    self.model,
                    short_id(&self.thread_id),
                    self.active_turn_id
                        .as_deref()
                        .map(short_id)
                        .unwrap_or("idle"),
                ))
                .style(Theme::header());
                f.render_widget(header, chunks[0]);

                let mut transcript_text = String::new();
                if self.messages.is_empty() {
                    transcript_text.push_str(
                        "\nNo transcript yet. Ask Roder to inspect, edit, or run something.\n",
                    );
                } else {
                    transcript_text.push_str(&self.messages.join("\n\n"));
                }

                if !self.events.is_empty() {
                    transcript_text.push_str("\n\n--- Events ---\n");
                    transcript_text.push_str(&self.events.join("\n"));
                }

                f.render_widget(Paragraph::new(transcript_text), chunks[1]);

                let input_text = if self.input.is_empty() {
                    "Ask Roder to work on this repo"
                } else {
                    self.input.as_str()
                };
                let composer =
                    Paragraph::new(input_text).block(Block::default().borders(Borders::ALL));
                f.render_widget(composer, chunks[2]);

                let footer = Paragraph::new(" Enter send  Ctrl-P menu  Ctrl-C interrupt  Esc quit")
                    .style(Theme::footer());
                f.render_widget(footer, chunks[3]);

                if self.show_ctrl_p_menu {
                    let area = f.size();
                    let menu_area = Rect::new(
                        area.width.saturating_sub(44) / 2,
                        area.height.saturating_sub(10) / 2,
                        44,
                        10,
                    );
                    let items = [
                        ListItem::new("1. Switch Provider/Model..."),
                        ListItem::new("2. Sign in with Codex..."),
                        ListItem::new("3. Settings..."),
                    ];
                    let menu = List::new(items)
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .title("Menu (Esc to close)"),
                        )
                        .highlight_style(Theme::highlight());
                    f.render_widget(Clear, menu_area);
                    f.render_stateful_widget(menu, menu_area, &mut self.menu_state);
                }
            })?;

            if event::poll(Duration::from_millis(50))?
                && let Event::Key(key) = event::read()?
            {
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('p') {
                    self.show_ctrl_p_menu = !self.show_ctrl_p_menu;
                } else if key.modifiers.contains(KeyModifiers::CONTROL)
                    && key.code == KeyCode::Char('c')
                {
                    if let Some(turn_id) = self.active_turn_id.clone() {
                        let client = self.client.clone();
                        let thread_id = self.thread_id.clone();
                        tokio::spawn(async move {
                            let params = InterruptTurnParams { thread_id, turn_id };
                            let _ = client
                                .send_request(JsonRpcRequest {
                                    jsonrpc: "2.0".to_string(),
                                    id: Some(serde_json::json!("interrupt")),
                                    method: "turns/interrupt".to_string(),
                                    params: Some(serde_json::to_value(params).unwrap()),
                                })
                                .await;
                        });
                    }
                } else if self.show_ctrl_p_menu {
                    match key.code {
                        KeyCode::Esc => self.show_ctrl_p_menu = false,
                        KeyCode::Up => {
                            let i = match self.menu_state.selected() {
                                Some(0) | None => 2,
                                Some(i) => i - 1,
                            };
                            self.menu_state.select(Some(i));
                        }
                        KeyCode::Down => {
                            let i = match self.menu_state.selected() {
                                Some(i) if i >= 2 => 0,
                                Some(i) => i + 1,
                                None => 0,
                            };
                            self.menu_state.select(Some(i));
                        }
                        KeyCode::Enter => {
                            self.messages
                                .push("System: menu action is not wired yet.".to_string());
                            self.show_ctrl_p_menu = false;
                        }
                        _ => {}
                    }
                } else {
                    match key.code {
                        KeyCode::Char(c) => self.input.push(c),
                        KeyCode::Backspace => {
                            self.input.pop();
                        }
                        KeyCode::Enter => {
                            let text = self.input.trim().to_string();
                            self.input.clear();
                            if text.is_empty() {
                                continue;
                            }
                            if text.starts_with('/') {
                                self.messages
                                    .push(format!("executed slash command: {}", text));
                                continue;
                            }
                            self.messages.push(format!("user: {}", text));
                            let params = StartTurnParams {
                                thread_id: self.thread_id.clone(),
                                message: text,
                                provider_override: None,
                                model_override: Some(self.model.clone()),
                            };
                            let client = self.client.clone();
                            tokio::spawn(async move {
                                let _ = client
                                    .send_request(JsonRpcRequest {
                                        jsonrpc: "2.0".to_string(),
                                        id: Some(serde_json::json!("turns/start")),
                                        method: "turns/start".to_string(),
                                        params: Some(serde_json::to_value(params).unwrap()),
                                    })
                                    .await;
                            });
                        }
                        KeyCode::Esc => break,
                        _ => {}
                    }
                }
            }

            while let Ok(envelope) = rx.try_recv() {
                self.events
                    .push(format!("{} #{}", envelope.kind, envelope.seq));
                if self.events.len() > 12 {
                    self.events.remove(0);
                }

                match envelope.event {
                    RoderEvent::TurnStarted(ev) => self.active_turn_id = Some(ev.turn_id),
                    RoderEvent::TurnCompleted(ev)
                        if self.active_turn_id.as_deref() == Some(&ev.turn_id) =>
                    {
                        self.active_turn_id = None;
                    }
                    RoderEvent::TurnInterrupted(ev)
                        if self.active_turn_id.as_deref() == Some(&ev.turn_id) =>
                    {
                        self.active_turn_id = None;
                    }
                    RoderEvent::InferenceEventReceived(ev) => {
                        if let roder_api::inference::InferenceEvent::MessageDelta(delta) = ev.event
                        {
                            if let Some(last) = self.messages.last_mut()
                                && last.starts_with("assistant: ")
                            {
                                last.push_str(&delta.text);
                                continue;
                            }
                            self.messages.push(format!("assistant: {}", delta.text));
                        }
                    }
                    RoderEvent::TurnFailed(ev) => {
                        self.messages.push(format!("error: {}", ev.error))
                    }
                    _ => {}
                }
            }
        }

        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
        terminal.show_cursor()?;

        Ok(())
    }
}

fn short_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}
