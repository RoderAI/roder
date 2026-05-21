use super::session_resume::{agents_list, commands_list, threads_list};
use super::*;
use crate::palette::{
    PaletteAction, collect_entries, cycle_source_filter,
    index::{PaletteMatch, search as search_palette},
    processes::process_source,
    render::palette_list,
    skills::skill_source,
    sources::{
        agent_source, command_source, marketplace_source, media_source, memories_source,
        mode_source, model_source, remote_source, roadmap_source, runner_source, session_source,
        settings_source, theme_source,
        workflow_import_source,
    },
};
use crate::theme::{discover_themes, discovery::default_directories};
use roder_protocol::MarketplacesListResult;

pub(super) fn is_palette_open_key(key: crossterm::event::KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('k')
}

fn is_palette_close_key(key: crossterm::event::KeyEvent) -> bool {
    key.code == KeyCode::Esc
}

impl<C> TuiApp<C>
where
    C: AppClient,
{
    pub(super) async fn open_palette(&mut self) {
        self.show_provider_popup = false;
        self.palette_query.clear();
        self.palette_source_filter = None;
        self.populate_palette().await;
    }

    pub(super) async fn open_resume_palette(&mut self) {
        self.show_provider_popup = false;
        self.palette_query.clear();
        self.palette_source_filter = Some("sessions".to_string());
        self.populate_palette().await;
    }

    async fn populate_palette(&mut self) {
        if let Ok(commands) = commands_list(&self.client).await {
            self.command_catalog = commands;
        }

        let sessions = match threads_list(&self.client).await {
            Ok(sessions) => sessions,
            Err(err) => {
                self.push_event(format!("thread/list unavailable: {err}"));
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
        let settings = match self.settings_get().await {
            Ok(settings) => Some(settings),
            Err(err) => {
                self.push_event(format!("settings/get unavailable: {err}"));
                None
            }
        };
        let runners = match self.runners_list().await {
            Ok(runners) => Some(runners),
            Err(err) => {
                self.push_event(format!("runners/list unavailable: {err}"));
                None
            }
        };
        let marketplaces = match self.marketplaces_list().await {
            Ok(marketplaces) => Some(marketplaces),
            Err(err) => {
                self.push_event(format!("marketplaces/list unavailable: {err}"));
                None
            }
        };
        let skills = match self.skills_list().await {
            Ok(skills) => Some(skills),
            Err(err) => {
                self.push_event(format!("skills/list unavailable: {err}"));
                None
            }
        };
        let processes = match self.processes_list(true).await {
            Ok(processes) => Some(processes),
            Err(err) => {
                self.push_event(format!("processes/list unavailable: {err}"));
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
        if self.palette_source_enabled("settings")
            && let Some(settings) = settings.as_ref()
        {
            sources.push(settings_source(&settings.web_search, &settings.search_index));
        }
        if self.palette_source_enabled("runners")
            && let Some(runners) = runners.as_ref()
        {
            sources.push(runner_source(runners));
        }
        if self.palette_source_enabled("models")
            && let Some(providers) = providers.as_ref()
        {
            sources.push(model_source(providers));
        }
        if self.palette_source_enabled("themes") {
            let entries = discover_themes(&default_directories());
            sources.push(theme_source(&entries, self.active_theme_id.as_deref()));
        }
        if self.palette_source_enabled("workflow-imports") {
            sources.push(workflow_import_source());
        }
        if self.palette_source_enabled("marketplaces")
            && let Some(marketplaces) = marketplaces.as_ref()
        {
            sources.push(marketplace_source(&marketplaces.marketplaces));
        }
        if self.palette_source_enabled("skills")
            && let Some(skills) = skills.as_ref()
        {
            sources.push(skill_source(&skills.skills));
        }
        if self.palette_source_enabled("media") {
            sources.push(media_source());
        }
        if self.palette_source_enabled("memories") {
            sources.push(memories_source());
        }
        if self.palette_source_enabled("processes")
            && let Some(processes) = processes.as_ref()
        {
            sources.push(process_source(&processes.processes));
        }
        if self.palette_source_enabled("remote") {
            sources.push(remote_source());
        }
        if self.palette_source_enabled("roadmaps") {
            sources.push(roadmap_source());
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
                    reasoning: None,
                    thread_id: Some(self.focused_thread_id().to_string()),
                })
                .await;
            }
            PaletteAction::SetPolicyMode(mode) => {
                self.set_policy_mode(mode, "palette mode switcher").await;
            }
            PaletteAction::SetWebSearchMode(mode) => {
                self.set_web_search_mode(mode).await;
            }
            PaletteAction::SetSearchIndexEnabled(enabled) => {
                self.set_search_index_enabled(enabled).await;
            }
            PaletteAction::SetSkillEnabled { selector, enabled } => {
                self.set_skill_enabled(selector, enabled).await;
            }
            PaletteAction::SetSkillExposure { selector, exposure } => {
                self.set_skill_exposure(selector, exposure).await;
            }
            PaletteAction::SelectRunner {
                destination_id,
                provider_id,
            } => {
                self.select_runner(destination_id, provider_id).await;
            }
            PaletteAction::InsertComposerText(text) => {
                self.composer = composer_textarea(self.theme);
                self.composer.insert_str(text);
            }
            PaletteAction::SetTheme(id) => {
                self.apply_theme_by_id(&id);
            }
            PaletteAction::OpenPluginBrowser => {
                self.open_plugin_browser().await;
            }
            PaletteAction::OpenSkillsManager => {
                self.palette_query.clear();
                self.palette_source_filter = Some("skills".to_string());
                self.populate_palette().await;
            }
            PaletteAction::OpenRoadmapMode => {
                self.enter_roadmap_mode(None);
            }
            PaletteAction::ShowAutomationsStatus => {
                self.show_automations_status().await;
            }
            PaletteAction::ShowProcesses => {
                self.show_processes(false).await;
            }
            PaletteAction::ShowProcessDetail(process_id) => {
                self.show_process_detail(&process_id).await;
            }
            PaletteAction::StopProcess(process_id) => {
                self.stop_process(&process_id, Some("palette stop")).await;
            }
        }
    }

    /// Live-switch the active theme. Reloads the stylesheet, restyles every
    /// composed widget, and persists the choice for next launch. Failures
    /// (unknown id, parse error, unwritable state file) surface in the event
    /// log instead of crashing.
    pub(super) fn apply_theme_by_id(&mut self, id: &str) {
        match crate::theme::load_theme_by_id(&default_directories(), id) {
            Some(overrides) => {
                let new_theme = Theme::for_terminal().with_overrides(&overrides);
                self.theme = new_theme;
                self.composer = composer_textarea(self.theme);
                self.active_theme_id = Some(id.to_string());
                if let Err(err) = crate::theme::write_active_theme(id) {
                    self.push_event(format!("theme: persist failed: {err}"));
                }
                self.push_event(format!("theme: switched to {id}"));
            }
            None => {
                self.push_event(format!("theme: cannot load {id} (not found or bad CSS)"));
            }
        }
    }

    fn palette_source_enabled(&self, source_id: &str) -> bool {
        self.enabled_palette_sources.is_empty() || self.enabled_palette_sources.contains(source_id)
    }

    async fn marketplaces_list(&self) -> anyhow::Result<MarketplacesListResult> {
        let res = self
            .client
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(serde_json::json!("marketplaces/list")),
                method: "marketplaces/list".to_string(),
                params: None,
            })
            .await;
        decode_response(res)
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
