//! Editing the shared GPT/Codex context budget without restarting the TUI.
use super::*;
use roder_api::catalog::effective_model_context_window;
use roder_protocol::{SettingsSetModelContextWindowParams, SettingsSetModelContextWindowResult};

#[derive(Debug, Default)]
pub(super) struct ContextSettings {
    pub saved: Option<u32>,
    input: String,
    error: Option<String>,
}

impl ContextSettings {
    pub fn new(saved: Option<u32>) -> Self {
        Self {
            saved,
            ..Self::default()
        }
    }

    pub fn begin_edit(&mut self) {
        self.input = self
            .saved
            .map(|value| value.to_string())
            .unwrap_or_default();
        self.error = None;
    }

    pub fn paste(&mut self, text: &str) {
        self.input.push_str(text.trim());
        self.error = None;
    }

    pub fn effective_window(
        &self,
        provider: &str,
        model: &str,
        fallback: Option<u32>,
    ) -> Option<u32> {
        effective_model_context_window(provider, model, self.saved).or(fallback)
    }

    fn parse(&self) -> Result<Option<u32>, &'static str> {
        if self.input.trim().is_empty() {
            return Ok(None);
        }
        self.input
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|value| *value > 0)
            .map(Some)
            .ok_or("Enter a positive whole number up to 4294967295, or clear to use default.")
    }
}

fn tokens(value: u32) -> String {
    let digits = value.to_string();
    let mut formatted = String::new();
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            formatted.push(',');
        }
        formatted.push(ch);
    }
    formatted
}

pub(super) fn setting_label(value: Option<u32>) -> String {
    format!(
        "GPT/Codex context window: {}",
        value.map_or_else(
            || "default (500,000 tokens)".to_string(),
            |v| { format!("{} tokens", tokens(v)) }
        )
    )
}

impl<C: AppClient> TuiApp<C> {
    pub(super) fn render_context_settings(&self, frame: &mut Frame<'_>, area: Rect) {
        let area = centered_rect(area, area.width.min(76), area.height.min(14));
        let block = Block::default()
            .title(" GPT/Codex context window ")
            .borders(Borders::ALL)
            .style(self.theme.dialog_surface());
        let inner = block.inner(area);
        let current = self.model_context_window.map_or_else(
            || "unknown".to_string(),
            |value| format!("{} tokens", tokens(value)),
        );
        let maximum = lookup_model_for_provider(&self.provider, &self.model)
            .map(|model| format!("{} tokens", tokens(model.max_context_window)))
            .unwrap_or_else(|| "provider-specific".to_string());
        let text = vec![
            Line::from(setting_label(self.context_settings.saved)),
            Line::from(format!("Current model: {current}")),
            Line::from(format!("Current model maximum: {maximum}")),
            Line::from("Only native OpenAI/Codex GPT models; capped at each model's maximum."),
            Line::from(""),
            Line::from(format!("Tokens: {}_", self.context_settings.input)),
            Line::styled(
                "Empty = catalog default (up to 500,000 tokens).",
                self.theme.muted(),
            ),
            Line::from(""),
            Line::styled(
                self.context_settings.error.as_deref().unwrap_or(""),
                self.theme.error(),
            ),
            Line::styled(
                "Enter save | Ctrl+R reset to default | Ctrl+U clear | Esc back",
                self.theme.muted(),
            ),
        ];
        frame.render_widget(Clear, area);
        frame.render_widget(block, area);
        frame.render_widget(
            Paragraph::new(text)
                .style(self.theme.dialog_surface())
                .wrap(Wrap { trim: false }),
            inner,
        );
    }

    pub(super) async fn handle_context_settings_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.open_settings_submenu(),
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.save_context_window(None).await;
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.context_settings.input.clear();
                self.context_settings.error = None;
            }
            KeyCode::Backspace => {
                self.context_settings.input.pop();
                self.context_settings.error = None;
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.context_settings.input.push(ch);
                self.context_settings.error = None;
            }
            KeyCode::Enter => match self.context_settings.parse() {
                Ok(value) => self.save_context_window(value).await,
                Err(message) => self.context_settings.error = Some(message.to_string()),
            },
            _ => {}
        }
    }

    async fn save_context_window(&mut self, value: Option<u32>) {
        let response = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("settings/set_model_context_window")),
                method: "settings/set_model_context_window".to_string(),
                params: Some(
                    serde_json::to_value(SettingsSetModelContextWindowParams {
                        model_context_window: value,
                    })
                    .expect("context setting serializes"),
                ),
            })
            .await;
        match decode_response::<SettingsSetModelContextWindowResult>(response) {
            Ok(result) => {
                self.context_settings.saved = result.model_context_window;
                // Apply locally first: a failed list refresh must not leave the
                // counter stale after a successful config write.
                self.model_context_window = self.context_settings.effective_window(
                    &self.provider,
                    &self.model,
                    context_window_for_provider_model(&self.provider, &self.model),
                );
                if let Ok(list) = self.providers_list().await {
                    self.model_options = provider_options_from_list(&list);
                    self.model_context_window = self.context_settings.effective_window(
                        &self.provider,
                        &self.model,
                        context_window_from_options(
                            &self.model_options,
                            &self.provider,
                            &self.model,
                        ),
                    );
                }
                let effective = self.model_context_window.map_or_else(
                    || "unknown".to_string(),
                    |value| format!("{} tokens", tokens(value)),
                );
                self.timeline.push_system(format!(
                    "{} (saved). Current model effective limit: {effective}.",
                    setting_label(result.model_context_window),
                ));
                self.open_settings_submenu();
            }
            Err(error) => {
                self.context_settings.error =
                    Some(format!("Could not save context window: {error}"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::events::EventEnvelope;
    use roder_protocol::{JsonRpcNotification, JsonRpcResponse};
    use std::sync::Mutex;

    #[derive(Clone)]
    struct ContextClient {
        requests: Arc<Mutex<Vec<JsonRpcRequest>>>,
        fail: bool,
        events: tokio::sync::broadcast::Sender<EventEnvelope>,
        notifications: tokio::sync::broadcast::Sender<JsonRpcNotification>,
    }

    impl ContextClient {
        fn new(fail: bool) -> Self {
            Self {
                requests: Arc::new(Mutex::new(Vec::new())),
                fail,
                events: tokio::sync::broadcast::channel(16).0,
                notifications: tokio::sync::broadcast::channel(16).0,
            }
        }
    }

    #[async_trait::async_trait]
    impl AppClient for ContextClient {
        type EventReceiver = tokio::sync::broadcast::Receiver<EventEnvelope>;
        type NotificationReceiver = tokio::sync::broadcast::Receiver<JsonRpcNotification>;

        async fn send_request(&self, request: JsonRpcRequest) -> JsonRpcResponse {
            self.requests.lock().unwrap().push(request.clone());
            let success = !self.fail && request.method == "settings/set_model_context_window";
            JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: request.id,
                result: success.then(|| {
                    serde_json::json!({
                        "model_context_window": request.params.unwrap()["model_context_window"],
                        "config_path": "/test/config.toml",
                    })
                }),
                error: (!success).then(|| roder_protocol::JsonRpcError {
                    code: -32000,
                    message: "Config write denied".to_string(),
                    data: None,
                }),
            }
        }

        fn subscribe_events(&self) -> Self::EventReceiver {
            self.events.subscribe()
        }

        fn subscribe_notifications(&self) -> Self::NotificationReceiver {
            self.notifications.subscribe()
        }
    }

    fn app(client: ContextClient) -> TuiApp<ContextClient> {
        let server = Arc::new(AppServer::new(Arc::new(
            roder_core::Runtime::fake().unwrap(),
        )));
        let mut app = super::super::tests::test_app_with_client(client, server);
        app.provider = "openai".to_string();
        app.model = "gpt-6.2-sol".to_string();
        app.model_context_window = Some(500_000);
        app
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn context_input_accepts_arbitrary_positive_numbers_and_reset() {
        let mut settings = ContextSettings::new(Some(537_321));
        settings.begin_edit();
        assert_eq!(settings.parse(), Ok(Some(537_321)));
        for invalid in ["0", "-1", "500k", "1.5", "4294967296"] {
            settings.input = invalid.to_string();
            assert!(settings.parse().is_err(), "{invalid}");
        }
        settings.input.clear();
        assert_eq!(settings.parse(), Ok(None));
    }

    #[test]
    fn context_labels_and_effective_windows_are_readable_and_scoped() {
        assert_eq!(tokens(537_321), "537,321");
        assert!(setting_label(None).contains("default (500,000 tokens)"));
        let settings = ContextSettings::new(Some(537_321));
        assert_eq!(
            settings.effective_window("openai", "gpt-6.2-sol", None),
            Some(537_321)
        );
        assert_eq!(
            settings.effective_window("codex", "gpt-6.2-astra", None),
            Some(537_321)
        );
        assert_eq!(
            settings.effective_window("anthropic", "claude-sonnet-4-6", None),
            context_window_for_provider_model("anthropic", "claude-sonnet-4-6")
        );
    }

    #[tokio::test]
    async fn context_settings_navigation_input_save_and_reset_refresh_counter() {
        let client = ContextClient::new(false);
        let mut app = app(client.clone());
        app.show_provider_popup = true;
        app.open_settings_submenu();
        let index = app
            .provider_menu_items
            .iter()
            .position(|item| matches!(item, ProviderMenuItem::ContextWindowSettings(None)))
            .unwrap();
        app.select_provider_menu_index(Some(index));
        app.select_current_provider_menu_item().await;
        assert_eq!(
            app.provider_popup_screen,
            ProviderPopupScreen::ContextWindow
        );
        app.handle_paste("537321".to_string());
        app.handle_context_settings_key(key(KeyCode::Enter)).await;
        assert_eq!(app.context_settings.saved, Some(537_321));
        assert_eq!(app.context_window_counter().unwrap().max_tokens, 537_321);
        assert_eq!(app.provider_popup_screen, ProviderPopupScreen::Settings);
        {
            let requests = client.requests.lock().unwrap();
            assert_eq!(requests[0].method, "settings/set_model_context_window");
            assert_eq!(
                requests[0].params.as_ref().unwrap()["model_context_window"],
                537_321
            );
        }

        app.provider_popup_screen = ProviderPopupScreen::ContextWindow;
        app.context_settings.begin_edit();
        app.handle_context_settings_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL))
            .await;
        assert_eq!(app.context_settings.saved, None);
        assert_eq!(app.context_window_counter().unwrap().max_tokens, 500_000);
        assert_eq!(
            client.requests.lock().unwrap()[2].params.as_ref().unwrap()["model_context_window"],
            serde_json::Value::Null
        );
    }

    #[tokio::test]
    async fn context_validation_and_rpc_errors_preserve_input_and_previous_limit() {
        let client = ContextClient::new(true);
        let mut app = app(client.clone());
        app.provider_popup_screen = ProviderPopupScreen::ContextWindow;
        app.context_settings.input = "0".to_string();
        app.handle_context_settings_key(key(KeyCode::Enter)).await;
        assert!(
            app.context_settings
                .error
                .as_deref()
                .unwrap()
                .contains("positive whole number")
        );
        assert!(client.requests.lock().unwrap().is_empty());
        app.handle_context_settings_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL))
            .await;
        for ch in "612345".chars() {
            app.handle_context_settings_key(key(KeyCode::Char(ch)))
                .await;
        }
        app.handle_context_settings_key(key(KeyCode::Enter)).await;
        assert!(
            app.context_settings
                .error
                .as_deref()
                .unwrap()
                .contains("Config write denied")
        );
        assert_eq!(app.context_settings.input, "612345");
        assert_eq!(app.model_context_window, Some(500_000));
        assert_eq!(app.context_settings.saved, None);
        assert_eq!(
            app.provider_popup_screen,
            ProviderPopupScreen::ContextWindow
        );
        app.handle_context_settings_key(key(KeyCode::Backspace))
            .await;
        assert_eq!(app.context_settings.input, "61234");
        assert_eq!(app.context_settings.error, None);
        app.handle_context_settings_key(key(KeyCode::Esc)).await;
        assert_eq!(app.provider_popup_screen, ProviderPopupScreen::Settings);
    }

    #[tokio::test]
    async fn context_save_preserves_preference_but_caps_current_model_only() {
        let mut app = app(ContextClient::new(false));
        app.save_context_window(Some(1_000_000)).await;
        assert_eq!(app.context_settings.saved, Some(1_000_000));
        assert_eq!(app.model_context_window, Some(872_000));
        app.provider = "anthropic".to_string();
        app.model = "claude-sonnet-4-6".to_string();
        let default = context_window_for_provider_model(&app.provider, &app.model);
        app.save_context_window(Some(300_000)).await;
        assert_eq!(app.model_context_window, default);
    }

    #[test]
    fn context_editor_renders_value_current_limit_and_actions() {
        let mut app = app(ContextClient::new(false));
        app.context_settings = ContextSettings::new(Some(537_321));
        app.context_settings.begin_edit();
        app.model_context_window = Some(537_321);
        let backend = ratatui::backend::TestBackend::new(80, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| app.render_context_settings(frame, frame.area()))
            .unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Current model: 537,321 tokens"));
        assert!(text.contains("Tokens: 537321_"));
        assert!(text.contains("Ctrl+R reset to default"));
    }
}
