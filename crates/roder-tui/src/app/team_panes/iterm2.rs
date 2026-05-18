use super::{
    PaneBackend, PaneBackendAvailability, PaneCommand, PaneEnvironment, PaneHandle, PaneMember,
};

#[derive(Debug, Clone, Eq, PartialEq)]
pub(super) struct Iterm2PaneBackend {
    command: String,
}

impl Iterm2PaneBackend {
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
        }
    }
}

impl PaneBackend for Iterm2PaneBackend {
    fn detect(&self, env: &dyn PaneEnvironment) -> PaneBackendAvailability {
        if env.var("TERM_PROGRAM").as_deref() != Some("iTerm.app") {
            return PaneBackendAvailability::Unavailable(
                "iTerm2 display mode requires TERM_PROGRAM=iTerm.app".to_string(),
            );
        }
        if !env.command_available(&self.command) {
            return PaneBackendAvailability::Unavailable(format!(
                "iTerm2 display mode requires {}",
                self.command
            ));
        }
        PaneBackendAvailability::Available
    }

    fn create_member_pane(&self, member: &PaneMember) -> PaneCommand {
        let mut args = vec![
            "split-pane".to_string(),
            "--horizontal".to_string(),
            "--".to_string(),
            member.roder_command.clone(),
        ];
        args.extend(member.attach_args());
        PaneCommand::new(self.command.clone(), args)
    }

    fn focus_member_pane(&self, pane: &PaneHandle) -> PaneCommand {
        PaneCommand::new(
            self.command.clone(),
            vec!["select-pane".to_string(), pane.id.clone()],
        )
    }

    fn close_member_pane(&self, pane: &PaneHandle) -> PaneCommand {
        PaneCommand::new(
            self.command.clone(),
            vec!["close-pane".to_string(), pane.id.clone()],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::team_panes::{PaneBackend, PaneMember};

    #[test]
    fn iterm_create_member_pane_uses_attach_command() {
        let backend = Iterm2PaneBackend::new("it2");
        let command = backend.create_member_pane(&PaneMember {
            team_id: "team-a".to_string(),
            member_id: "member-1".to_string(),
            roder_command: "roder".to_string(),
        });

        assert_eq!(command.program, "it2");
        assert_eq!(
            command.args,
            vec![
                "split-pane",
                "--horizontal",
                "--",
                "roder",
                "team",
                "attach",
                "--team",
                "team-a",
                "--member",
                "member-1"
            ]
        );
    }
}
