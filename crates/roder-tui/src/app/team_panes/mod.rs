use roder_api::teams::{AgentTeamDisplayMode, TeamId, TeamMemberId};

pub mod iterm2;
pub mod tmux;

#[derive(Debug, Clone, Eq, PartialEq)]
pub(super) struct PaneCommand {
    pub program: String,
    pub args: Vec<String>,
}

impl PaneCommand {
    pub fn new(program: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(super) struct PaneMember {
    pub team_id: TeamId,
    pub member_id: TeamMemberId,
    pub roder_command: String,
}

impl PaneMember {
    pub fn attach_args(&self) -> Vec<String> {
        vec![
            "team".to_string(),
            "attach".to_string(),
            "--team".to_string(),
            self.team_id.clone(),
            "--member".to_string(),
            self.member_id.clone(),
        ]
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(super) struct PaneHandle {
    pub id: String,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(super) enum PaneBackendAvailability {
    Available,
    Unavailable(String),
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) enum ResolvedPaneBackend {
    InProcess,
    Tmux,
    Iterm2,
}

pub(super) trait PaneBackend {
    fn detect(&self, env: &dyn PaneEnvironment) -> PaneBackendAvailability;
    fn create_member_pane(&self, member: &PaneMember) -> PaneCommand;
    fn focus_member_pane(&self, pane: &PaneHandle) -> PaneCommand;
    fn close_member_pane(&self, pane: &PaneHandle) -> PaneCommand;
    fn cleanup_team(&self, panes: &[PaneHandle]) -> Vec<PaneCommand> {
        panes
            .iter()
            .map(|pane| self.close_member_pane(pane))
            .collect()
    }
}

pub(super) trait PaneEnvironment {
    fn var(&self, key: &str) -> Option<String>;
    fn command_available(&self, command: &str) -> bool;
}

pub(super) struct ProcessPaneEnvironment;

impl PaneEnvironment for ProcessPaneEnvironment {
    fn var(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }

    fn command_available(&self, command: &str) -> bool {
        std::process::Command::new(command)
            .arg("--version")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }
}

pub(super) fn resolve_display_mode(
    requested: AgentTeamDisplayMode,
    tmux: PaneBackendAvailability,
    iterm2: PaneBackendAvailability,
) -> (ResolvedPaneBackend, Option<String>) {
    match requested {
        AgentTeamDisplayMode::InProcess => (ResolvedPaneBackend::InProcess, None),
        AgentTeamDisplayMode::Tmux => match tmux {
            PaneBackendAvailability::Available => (ResolvedPaneBackend::Tmux, None),
            PaneBackendAvailability::Unavailable(reason) => {
                (ResolvedPaneBackend::InProcess, Some(reason))
            }
        },
        AgentTeamDisplayMode::Iterm2 => match iterm2 {
            PaneBackendAvailability::Available => (ResolvedPaneBackend::Iterm2, None),
            PaneBackendAvailability::Unavailable(reason) => {
                (ResolvedPaneBackend::InProcess, Some(reason))
            }
        },
        AgentTeamDisplayMode::Auto => match (tmux, iterm2) {
            (PaneBackendAvailability::Available, _) => (ResolvedPaneBackend::Tmux, None),
            (_, PaneBackendAvailability::Available) => (ResolvedPaneBackend::Iterm2, None),
            (
                PaneBackendAvailability::Unavailable(tmux_reason),
                PaneBackendAvailability::Unavailable(iterm_reason),
            ) => (
                ResolvedPaneBackend::InProcess,
                Some(format!("{tmux_reason}; {iterm_reason}")),
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::team_panes::iterm2::Iterm2PaneBackend;
    use crate::app::team_panes::tmux::TmuxPaneBackend;
    use std::collections::HashMap;

    #[test]
    fn auto_prefers_tmux_then_iterm_then_in_process() {
        let (mode, reason) = resolve_display_mode(
            AgentTeamDisplayMode::Auto,
            PaneBackendAvailability::Available,
            PaneBackendAvailability::Available,
        );
        assert_eq!(mode, ResolvedPaneBackend::Tmux);
        assert!(reason.is_none());

        let (mode, reason) = resolve_display_mode(
            AgentTeamDisplayMode::Auto,
            PaneBackendAvailability::Unavailable("tmux unavailable".to_string()),
            PaneBackendAvailability::Available,
        );
        assert_eq!(mode, ResolvedPaneBackend::Iterm2);
        assert!(reason.is_none());

        let (mode, reason) = resolve_display_mode(
            AgentTeamDisplayMode::Auto,
            PaneBackendAvailability::Unavailable("tmux unavailable".to_string()),
            PaneBackendAvailability::Unavailable("it2 unavailable".to_string()),
        );
        assert_eq!(mode, ResolvedPaneBackend::InProcess);
        assert_eq!(reason.as_deref(), Some("tmux unavailable; it2 unavailable"));
    }

    #[test]
    fn tmux_backend_detects_only_inside_tmux_with_command_available() {
        let backend = TmuxPaneBackend::new("tmux");
        let env = FakeEnv::new([("TMUX", "/tmp/tmux")], ["tmux"]);

        assert_eq!(backend.detect(&env), PaneBackendAvailability::Available);

        let missing_tmux_env = FakeEnv::new([], ["tmux"]);
        assert_eq!(
            backend.detect(&missing_tmux_env),
            PaneBackendAvailability::Unavailable("tmux display mode requires $TMUX".to_string())
        );
    }

    #[test]
    fn iterm_backend_requires_iterm_program_and_it2() {
        let backend = Iterm2PaneBackend::new("it2");
        let env = FakeEnv::new([("TERM_PROGRAM", "iTerm.app")], ["it2"]);

        assert_eq!(backend.detect(&env), PaneBackendAvailability::Available);

        let missing_it2 = FakeEnv::new([("TERM_PROGRAM", "iTerm.app")], []);
        assert_eq!(
            backend.detect(&missing_it2),
            PaneBackendAvailability::Unavailable("iTerm2 display mode requires it2".to_string())
        );
    }

    struct FakeEnv {
        vars: HashMap<String, String>,
        commands: Vec<String>,
    }

    impl FakeEnv {
        fn new<const V: usize, const C: usize>(
            vars: [(&str, &str); V],
            commands: [&str; C],
        ) -> Self {
            Self {
                vars: vars
                    .into_iter()
                    .map(|(key, value)| (key.to_string(), value.to_string()))
                    .collect(),
                commands: commands.into_iter().map(str::to_string).collect(),
            }
        }
    }

    impl PaneEnvironment for FakeEnv {
        fn var(&self, key: &str) -> Option<String> {
            self.vars.get(key).cloned()
        }

        fn command_available(&self, command: &str) -> bool {
            self.commands.iter().any(|candidate| candidate == command)
        }
    }
}
