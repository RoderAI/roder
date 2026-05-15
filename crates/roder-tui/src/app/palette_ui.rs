use super::*;
use crate::palette::{
    PaletteAction, collect_entries, cycle_source_filter,
    index::{PaletteMatch, search as search_palette},
    render::palette_list,
    sources::{agent_source, command_source, mode_source, model_source, session_source},
};

pub(super) fn is_palette_open_key(key: crossterm::event::KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('k')
}

fn is_palette_close_key(key: crossterm::event::KeyEvent) -> bool {
    key.code == KeyCode::Esc
}

impl TuiApp {
    pub(super) async fn open_palette(&mut self) {
        self.show_provider_popup = false;
        self.palette_query.clear();
        self.palette_source_filter = None;

        if let Ok(commands) = commands_list(&self.client).await {
            self.command_catalog = commands;
        }

        let sessions = match sessions_list(&self.client).await {
            Ok(sessions) => sessions,
            Err(err) => {
                self.push_event(format!("sessions/list unavailable: {err}"));
                Vec::new()
            }
        };
        let agents = match agents_list(&self.client).await {
            Ok(agents) => agents,
            Err(err) => {
                self.push_event(format!("agents/list unavailable: {err}"));
                Vec::new()
            }
        };
        let providers = match self.providers_list().await {
            Ok(providers) => Some(providers),
            Err(err) => {
                self.push_event(format!("providers/list unavailable: {err}"));
                None
            }
        };

        let mut sources = Vec::new();
        if self.palette_source_enabled("commands") {
            sources.push(command_source(&self.command_catalog));
        }
        if self.palette_source_enabled("sessions") {
            sources.push(session_source(&sessions));
        }
        if self.palette_source_enabled("agents") {
            sources.push(agent_source(&agents));
        }
        if self.palette_source_enabled("modes") {
            sources.push(mode_source(self.policy_mode));
        }
        if self.palette_source_enabled("models")
            && let Some(providers) = providers.as_ref()
        {
            sources.push(model_source(providers));
        }
        self.palette_entries = collect_entries(&sources);
        self.palette_state
            .select((!self.palette_entries.is_empty()).then_some(0));
        self.show_palette = true;
    }

    pub(super) async fn handle_palette_key(&mut self, key: crossterm::event::KeyEvent) {
        match key.code {
            _ if is_palette_close_key(key) => {
                self.show_palette = false;
            }
            KeyCode::Up => {
                self.move_palette_selection(-1);
            }
            KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_palette_selection(-1);
            }
            KeyCode::Down => {
                self.move_palette_selection(1);
            }
            KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_palette_selection(1);
            }
            KeyCode::Tab => {
                self.palette_source_filter = cycle_source_filter(
                    self.palette_source_filter.as_deref(),
                    &self.palette_entries,
                );
                self.clamp_palette_selection();
            }
            KeyCode::Enter => {
                self.execute_selected_palette_action().await;
            }
            KeyCode::Backspace => {
                self.palette_query.pop();
                self.clamp_palette_selection();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.palette_query.push(c);
                self.clamp_palette_selection();
            }
            _ => {}
        }
    }

    pub(super) fn render_palette_popup(&mut self, f: &mut Frame<'_>, area: Rect) {
        let matches = self.palette_matches();
        let list = palette_list(
            &matches,
            &self.palette_query,
            self.palette_source_filter.as_deref(),
            self.theme.palette(),
        );
        let selected = self
            .palette_state
            .selected()
            .unwrap_or(0)
            .min(matches.len().saturating_sub(1));
        if matches.is_empty() {
            self.palette_state.select(None);
        } else {
            self.palette_state.select(Some(selected));
        }
        let palette_area = centered_rect(area, area.width.min(86), area.height.min(18));
        f.render_widget(Clear, palette_area);
        f.render_stateful_widget(list, palette_area, &mut self.palette_state);
    }

    fn palette_matches(&self) -> Vec<PaletteMatch<'_>> {
        search_palette(
            &self.palette_entries,
            &self.palette_query,
            self.palette_source_filter.as_deref(),
        )
    }

    fn move_palette_selection(&mut self, delta: isize) {
        let count = self.palette_matches().len();
        if count == 0 {
            self.palette_state.select(None);
            return;
        }
        let current = self.palette_state.selected().unwrap_or(0);
        let next = (current as isize + delta).rem_euclid(count as isize) as usize;
        self.palette_state.select(Some(next));
    }

    fn clamp_palette_selection(&mut self) {
        let count = self.palette_matches().len();
        if count == 0 {
            self.palette_state.select(None);
        } else {
            let selected = self.palette_state.selected().unwrap_or(0).min(count - 1);
            self.palette_state.select(Some(selected));
        }
    }

    async fn execute_selected_palette_action(&mut self) {
        let matches = self.palette_matches();
        let selected = self.palette_state.selected().unwrap_or(0);
        let action = matches
            .get(selected)
            .map(|matched| matched.entry.action.clone());
        drop(matches);

        let Some(action) = action else {
            self.show_palette = false;
            return;
        };
        self.show_palette = false;
        self.execute_palette_action(action).await;
    }

    async fn execute_palette_action(&mut self, action: PaletteAction) {
        match action {
            PaletteAction::SendCommand(command) => {
                let invocation = format!("/{command}");
                if !self.try_run_slash_command(&invocation).await {
                    self.composer = composer_textarea(self.theme);
                    self.composer.insert_str(format!("/{command} "));
                }
            }
            PaletteAction::SwitchSession(thread_id) => {
                self.load_session(thread_id).await;
            }
            PaletteAction::SwitchModel { provider, model } => {
                self.select_provider_model_params(ProviderSelectParams {
                    provider,
                    model: Some(model),
                })
                .await;
            }
            PaletteAction::SetPolicyMode(mode) => {
                self.set_policy_mode(mode, "palette mode switcher").await;
            }
            PaletteAction::InsertComposerText(text) => {
                self.composer = composer_textarea(self.theme);
                self.composer.insert_str(text);
            }
        }
    }

    fn palette_source_enabled(&self, source_id: &str) -> bool {
        self.enabled_palette_sources.is_empty() || self.enabled_palette_sources.contains(source_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_open_and_close_keys_are_modal_safe() {
        assert!(is_palette_open_key(crossterm::event::KeyEvent::new(
            KeyCode::Char('k'),
            KeyModifiers::CONTROL,
        )));
        assert!(!is_palette_open_key(crossterm::event::KeyEvent::new(
            KeyCode::Char('k'),
            KeyModifiers::NONE,
        )));
        assert!(is_palette_close_key(crossterm::event::KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
    }
}
