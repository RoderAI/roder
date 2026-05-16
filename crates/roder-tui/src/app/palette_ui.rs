use super::*;
use crate::palette::{
    PaletteAction, collect_entries, cycle_source_filter,
    index::{PaletteMatch, search as search_palette},
    render::palette_list,
    sources::{agent_source, command_source, mode_source, model_source, session_source},
};
use roder_api::interactive::{HoverCursor, InteractiveRegion, RegionKind};

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
        let palette_area = palette_popup_area(area);
        f.render_widget(Clear, palette_area);
        f.render_stateful_widget(list, palette_area, &mut self.palette_state);
    }

    pub(super) fn palette_item_regions(&self, area: Rect) -> Vec<InteractiveRegion> {
        let matches = self.palette_matches();
        palette_item_regions_for_matches(&matches, palette_popup_area(area))
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

    pub(super) async fn execute_palette_item(&mut self, source_id: &str, item_id: &str) {
        let matches = self.palette_matches();
        let selected = matches.iter().position(|matched| {
            matched.entry.source_id == source_id && matched.entry.item.id == item_id
        });
        let action = selected
            .and_then(|idx| matches.get(idx).map(|matched| matched.entry.action.clone()))
            .or_else(|| {
                self.palette_entries
                    .iter()
                    .find(|entry| entry.source_id == source_id && entry.item.id == item_id)
                    .map(|entry| entry.action.clone())
            });
        drop(matches);
        if let Some(selected) = selected {
            self.palette_state.select(Some(selected));
        }

        let Some(action) = action else {
            self.show_palette = false;
            self.push_event(format!("palette item unavailable: {source_id}/{item_id}"));
            return;
        };
        self.show_palette = false;
        self.push_event(format!("palette item selected: {source_id}/{item_id}"));
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

fn palette_popup_area(area: Rect) -> Rect {
    centered_rect(area, area.width.min(86), area.height.min(18))
}

fn palette_item_regions_for_matches(
    matches: &[PaletteMatch<'_>],
    palette_area: Rect,
) -> Vec<InteractiveRegion> {
    let rows = usize::from(palette_area.height.saturating_sub(2)).min(12);
    matches
        .iter()
        .take(rows)
        .enumerate()
        .map(|(idx, matched)| InteractiveRegion {
            id: format!(
                "palette:{}:{}",
                matched.entry.source_id, matched.entry.item.id
            ),
            rect: roder_api::interactive::RegionRect {
                x: palette_area.x.saturating_add(1),
                y: palette_area.y.saturating_add(1 + idx as u16),
                width: palette_area.width.saturating_sub(2),
                height: 1,
            },
            z: 20,
            kind: RegionKind::PaletteItem {
                source_id: matched.entry.source_id.clone(),
                item_id: matched.entry.item.id.clone(),
            },
            hover_cursor: HoverCursor::Pointer,
            keyboard_binding: None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palette::{PaletteEntry, PaletteItem};

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

    #[test]
    fn palette_item_regions_cover_visible_popup_rows() {
        let entries = vec![
            palette_entry("commands", "review"),
            palette_entry("models", "gpt-5"),
        ];
        let matches = search_palette(&entries, "", None);

        let regions = palette_item_regions_for_matches(&matches, Rect::new(10, 5, 40, 6));

        assert_eq!(regions.len(), 2);
        assert_eq!(regions[0].id, "palette:commands:review");
        assert_eq!(regions[0].rect.x, 11);
        assert_eq!(regions[0].rect.y, 6);
        assert_eq!(regions[0].rect.width, 38);
        assert_eq!(
            regions[1].kind,
            RegionKind::PaletteItem {
                source_id: "models".to_string(),
                item_id: "gpt-5".to_string()
            }
        );
    }

    fn palette_entry(source_id: &str, item_id: &str) -> PaletteEntry {
        PaletteEntry {
            source_id: source_id.to_string(),
            source_label: source_id.to_string(),
            item: PaletteItem {
                id: item_id.to_string(),
                title: item_id.to_string(),
                subtitle: None,
                keywords: Vec::new(),
                icon: None,
            },
            action: PaletteAction::InsertComposerText(item_id.to_string()),
        }
    }
}
