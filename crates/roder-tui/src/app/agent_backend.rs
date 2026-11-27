use super::*;

/// The agent runtime executing turns. A backend is bound when the app-server
/// starts, so switching relaunches Roder with `--backend <id>` instead of
/// swapping runtimes in place (threads and model catalogs are backend-owned).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AgentBackendChoice {
    #[default]
    Roder,
    Codex,
    Antigravity,
}

impl AgentBackendChoice {
    pub const ALL: [Self; 3] = [Self::Roder, Self::Codex, Self::Antigravity];

    pub fn id(self) -> &'static str {
        match self {
            Self::Roder => "roder",
            Self::Codex => "codex",
            Self::Antigravity => "antigravity",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|choice| choice.id() == id)
    }

    fn description(self) -> &'static str {
        match self {
            Self::Roder => "Roder - native runtime with your configured providers",
            Self::Codex => "Codex - ChatGPT subscription via the Codex CLI",
            Self::Antigravity => "Antigravity - Google subscription via the agy CLI",
        }
    }

    pub(super) fn menu_label(self, active: bool) -> String {
        if active {
            format!("{} (active)", self.description())
        } else {
            self.description().to_string()
        }
    }
}

impl<C> TuiApp<C>
where
    C: AppClient,
{
    /// Record which backend this process was started with.
    pub fn with_agent_backend(mut self, backend: AgentBackendChoice) -> Self {
        self.agent_backend = backend;
        self
    }

    /// The backend the user chose to relaunch into, if they switched.
    pub fn requested_agent_backend(&self) -> Option<AgentBackendChoice> {
        self.requested_agent_backend
    }

    pub(super) fn open_agent_backend_submenu(&mut self) {
        self.provider_popup_screen = ProviderPopupScreen::AgentBackends;
        self.provider_menu_filter.clear();
        self.provider_menu_items = AgentBackendChoice::ALL
            .into_iter()
            .map(|choice| ProviderMenuItem::AgentBackend {
                choice,
                active: choice == self.agent_backend,
            })
            .chain(std::iter::once(ProviderMenuItem::Back))
            .collect();
        let selected = AgentBackendChoice::ALL
            .iter()
            .position(|choice| *choice == self.agent_backend)
            .unwrap_or(0);
        self.select_provider_menu_index(Some(selected));
    }

    pub(super) fn request_agent_backend_switch(&mut self, choice: AgentBackendChoice) {
        if choice == self.agent_backend {
            self.show_provider_popup = false;
            self.push_event(format!("already using the {} backend", choice.id()));
            return;
        }
        if self.active_turn_id.is_some() {
            self.record_error(
                "finish or interrupt the running turn before switching agent backends".into(),
            );
            return;
        }
        self.show_provider_popup = false;
        self.requested_agent_backend = Some(choice);
        self.push_event(format!("restarting with the {} backend", choice.id()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::test_app;

    #[test]
    fn backend_ids_round_trip() {
        for choice in AgentBackendChoice::ALL {
            assert_eq!(AgentBackendChoice::from_id(choice.id()), Some(choice));
        }
        assert_eq!(AgentBackendChoice::from_id("gemini"), None);
    }

    #[tokio::test]
    async fn backend_menu_requests_a_relaunch_into_the_chosen_backend() {
        let mut app = test_app();
        app.show_provider_popup = true;
        app.open_agent_backend_submenu();
        assert!(matches!(
            app.provider_menu_items.first(),
            Some(ProviderMenuItem::AgentBackend {
                choice: AgentBackendChoice::Roder,
                active: true
            })
        ));
        let codex = app
            .provider_menu_items
            .iter()
            .position(|item| {
                matches!(
                    item,
                    ProviderMenuItem::AgentBackend {
                        choice: AgentBackendChoice::Codex,
                        ..
                    }
                )
            })
            .unwrap();
        app.provider_state.select(Some(codex));
        app.select_current_provider_menu_item().await;

        assert!(!app.show_provider_popup);
        assert_eq!(
            app.requested_agent_backend(),
            Some(AgentBackendChoice::Codex)
        );
    }

    #[tokio::test]
    async fn backend_switch_waits_for_the_running_turn_and_ignores_the_active_one() {
        let mut app = test_app().with_agent_backend(AgentBackendChoice::Antigravity);
        app.request_agent_backend_switch(AgentBackendChoice::Antigravity);
        assert_eq!(app.requested_agent_backend(), None);

        app.active_turn_id = Some("turn-1".into());
        app.request_agent_backend_switch(AgentBackendChoice::Roder);
        assert_eq!(app.requested_agent_backend(), None);
    }
}
