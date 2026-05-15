use std::io::{self};
use std::time::Duration;
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    widgets::{Block, Borders, Paragraph},
    Terminal,
};
use roder_app_server::LocalAppClient;
use roder_protocol::{JsonRpcRequest, StartTurnParams};
use roder_api::events::{EventEnvelope, RoderEvent};
use tokio::sync::broadcast;

pub struct TuiApp {
    client: LocalAppClient,
    thread_id: String,
    model: String,
    input: String,
    messages: Vec<String>,
    events: Vec<String>,
    show_ctrl_p_menu: bool,
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
        let thread_id = if let Some(result) = res.result {
            result.get("thread_id").unwrap().as_str().unwrap().to_string()
        } else {
            "unknown".to_string()
        };

        Ok(Self {
            client,
            thread_id,
            model,
            input: String::new(),
            messages: Vec::new(),
            events: Vec::new(),
            show_ctrl_p_menu: false,
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
                        Constraint::Length(1), // Header
                        Constraint::Min(5),    // Transcript
                        Constraint::Length(3), // Composer
                        Constraint::Length(1), // Footer
                    ])
                    .split(f.size());

                let header = Paragraph::new(" gode  openai/gpt-4o high                      idle")
                    .style(ratatui::style::Style::default().bg(ratatui::style::Color::DarkGray).fg(ratatui::style::Color::White));
                f.render_widget(header, chunks[0]);

                let mut transcript_text = String::new();
                if self.messages.is_empty() {
                    transcript_text.push_str("\nNo transcript yet. Ask gode to inspect, edit, or run something.\n");
                } else {
                    transcript_text.push_str(&self.messages.join("\n\n"));
                }

                // Add event log optionally below
                if !self.events.is_empty() {
                    transcript_text.push_str("\n\n--- Events ---\n");
                    transcript_text.push_str(&self.events.join("\n"));
                }

                let transcript = Paragraph::new(transcript_text);
                f.render_widget(transcript, chunks[1]);

                let input_text = if self.input.is_empty() {
                    "Ask gode to work on this repo"
                } else {
                    self.input.as_str()
                };

                let composer = Paragraph::new(input_text)
                    .block(Block::default().borders(Borders::ALL));
                f.render_widget(composer, chunks[2]);

                let footer = Paragraph::new(" ready                                             errors 0  ctx 100%  scroll 0")
                    .style(ratatui::style::Style::default().bg(ratatui::style::Color::DarkGray).fg(ratatui::style::Color::White));
                f.render_widget(footer, chunks[3]);
            })?;

            if event::poll(Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    match key.code {
                        KeyCode::Char(c) => {
                            self.input.push(c);
                        }
                        KeyCode::Backspace => {
                            self.input.pop();
                        }
                        KeyCode::Enter => {
                            let text = self.input.clone();
                            self.input.clear();
                            self.messages.push(format!("▌ {}", text));

                            let params = StartTurnParams {
                                thread_id: self.thread_id.clone(),
                                message: text,
                                provider_override: None,
                                model_override: None,
                            };
                            let req = JsonRpcRequest {
                                jsonrpc: "2.0".to_string(),
                                id: Some(serde_json::json!(2)),
                                method: "turns/start".to_string(),
                                params: Some(serde_json::to_value(params).unwrap()),
                            };

                            let client = self.client.server.clone();
                            tokio::spawn(async move {
                                client.handle_request(req).await;
                            });
                        }
                        KeyCode::Esc => {
                            break;
                        }
                        _ => {}
                    }
                }
            }

            // drain events
            while let Ok(envelope) = rx.try_recv() {
                self.events.push(format!("{:?}", envelope.event));
                if self.events.len() > 10 {
                    self.events.remove(0);
                }

                // Append text delta if it's a message delta
                if let RoderEvent::InferenceEventReceived(ev) = envelope.event {
                    if let roder_api::inference::InferenceEvent::MessageDelta(delta) = ev.event {
                        if let Some(last) = self.messages.last_mut() {
                            if !last.starts_with("▌ ") {
                                last.push_str(&delta.text);
                            } else {
                                self.messages.push(delta.text.clone());
                            }
                        } else {
                            self.messages.push(delta.text.clone());
                        }
                    }
                }
            }
        }

        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
        terminal.show_cursor()?;

        Ok(())
    }
}