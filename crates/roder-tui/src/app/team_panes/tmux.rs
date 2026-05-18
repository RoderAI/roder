use super::{
    PaneBackend, PaneBackendAvailability, PaneCommand, PaneEnvironment, PaneHandle, PaneMember,
};

#[derive(Debug, Clone, Eq, PartialEq)]
pub(super) struct TmuxPaneBackend {
    command: String,
}

impl TmuxPaneBackend {
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
        }
    }
}

impl PaneBackend for TmuxPaneBackend {
    fn detect(&self, env: &dyn PaneEnvironment) -> PaneBackendAvailability {
        if env.var("TMUX").is_none() {
            return PaneBackendAvailability::Unavailable(
                "tmux display mode requires $TMUX".to_string(),
            );
        }
        if !env.command_available(&self.command) {
            return PaneBackendAvailability::Unavailable(format!(
                "tmux display mode requires {}",
                self.command
            ));
        }
        PaneBackendAvailability::Available
    }

    fn create_member_pane(&self, member: &PaneMember) -> PaneCommand {
        let mut args = vec![
            "split-window".to_string(),
            "-h".to_string(),
            "-P".to_string(),
            "-F".to_string(),
            "#{pane_id}".to_string(),
            "--".to_string(),
            member.roder_command.clone(),
        ];
        args.extend(member.attach_args());
        PaneCommand::new(self.command.clone(), args)
    }

    fn focus_member_pane(&self, pane: &PaneHandle) -> PaneCommand {
        PaneCommand::new(
            self.command.clone(),
            vec!["select-pane".to_string(), "-t".to_string(), pane.id.clone()],
        )
    }

    fn close_member_pane(&self, pane: &PaneHandle) -> PaneCommand {
        PaneCommand::new(
            self.command.clone(),
            vec!["kill-pane".to_string(), "-t".to_string(), pane.id.clone()],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::team_panes::{PaneBackend, PaneMember};

    #[test]
    fn tmux_create_member_pane_uses_attach_command_and_prints_pane_id() {
        let backend = TmuxPaneBackend::new("tmux");
        let command = backend.create_member_pane(&PaneMember {
            team_id: "team-a".to_string(),
            member_id: "member-1".to_string(),
            roder_command: "roder".to_string(),
        });

        assert_eq!(command.program, "tmux");
        assert_eq!(
            command.args,
            vec![
                "split-window",
                "-h",
                "-P",
                "-F",
                "#{pane_id}",
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

    #[test]
    fn tmux_cleanup_closes_only_recorded_panes() {
        let backend = TmuxPaneBackend::new("tmux");
        let commands = backend.cleanup_team(&[
            PaneHandle {
                id: "%1".to_string(),
            },
            PaneHandle {
                id: "%2".to_string(),
            },
        ]);

        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0].args, vec!["kill-pane", "-t", "%1"]);
        assert_eq!(commands[1].args, vec!["kill-pane", "-t", "%2"]);
    }
}
