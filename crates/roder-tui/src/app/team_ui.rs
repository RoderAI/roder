use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use roder_api::events::ThreadId;
use roder_api::teams::{
    TeamId, TeamMemberDescriptor, TeamMemberId, TeamMemberRole, TeamMemberStatus,
};

#[derive(Debug, Clone, Eq, PartialEq)]
pub(super) struct TeamUiMember {
    pub id: TeamMemberId,
    pub name: String,
    pub role: TeamMemberRole,
    pub thread_id: ThreadId,
    pub status: TeamMemberStatus,
    pub unread_count: usize,
}

impl From<TeamMemberDescriptor> for TeamUiMember {
    fn from(member: TeamMemberDescriptor) -> Self {
        Self {
            id: member.id,
            name: member.name,
            role: member.role,
            thread_id: member.thread_id,
            status: member.status,
            unread_count: 0,
        }
    }
}

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub(super) struct TeamUiState {
    team_id: Option<TeamId>,
    focused_index: usize,
    members: Vec<TeamUiMember>,
    focused_by_thread: HashMap<ThreadId, TeamMemberId>,
    show_task_list: bool,
}

impl TeamUiState {
    #[allow(dead_code)]
    pub fn is_active(&self) -> bool {
        self.team_id.is_some() && !self.members.is_empty()
    }

    #[allow(dead_code)]
    pub fn set_team(&mut self, team_id: TeamId, members: Vec<TeamMemberDescriptor>) {
        self.team_id = Some(team_id);
        self.members = members.into_iter().map(TeamUiMember::from).collect();
        self.focused_index = 0;
        self.rebuild_thread_index();
    }

    pub fn focused_member(&self) -> Option<&TeamUiMember> {
        self.members.get(self.focused_index)
    }

    pub fn focused_member_id(&self) -> Option<&str> {
        self.focused_member().map(|member| member.id.as_str())
    }

    pub fn focused_thread_id<'a>(&'a self, fallback: &'a str) -> &'a str {
        self.focused_member()
            .map(|member| member.thread_id.as_str())
            .unwrap_or(fallback)
    }

    pub fn member_id_for_thread(&self, thread_id: &str) -> Option<&str> {
        self.focused_by_thread.get(thread_id).map(String::as_str)
    }

    pub fn focus_next(&mut self) -> bool {
        self.cycle_focus(1)
    }

    pub fn focus_previous(&mut self) -> bool {
        self.cycle_focus(-1)
    }

    pub fn focus_member(&mut self, member_id: &str) -> bool {
        let Some(index) = self
            .members
            .iter()
            .position(|member| member.id == member_id)
        else {
            return false;
        };
        if let Some(member) = self.members.get_mut(index) {
            member.unread_count = 0;
        }
        self.focused_index = index;
        true
    }

    pub fn record_thread_activity(&mut self, thread_id: &str) {
        let Some(member_id) = self.focused_by_thread.get(thread_id) else {
            return;
        };
        let Some(index) = self
            .members
            .iter()
            .position(|member| member.id == *member_id)
        else {
            return;
        };
        if index != self.focused_index {
            self.members[index].unread_count = self.members[index].unread_count.saturating_add(1);
        }
    }

    pub fn set_member_status(&mut self, member_id: &str, status: TeamMemberStatus) {
        if let Some(member) = self
            .members
            .iter_mut()
            .find(|member| member.id == member_id)
        {
            member.status = status;
        }
    }

    #[allow(dead_code)]
    pub fn toggle_task_list(&mut self) {
        self.show_task_list = !self.show_task_list;
    }

    #[allow(dead_code)]
    pub fn show_task_list(&self) -> bool {
        self.show_task_list
    }

    pub fn focused_label(&self) -> Option<String> {
        let member = self.focused_member()?;
        let unread = self
            .members
            .iter()
            .filter(|candidate| candidate.id != member.id)
            .map(|candidate| candidate.unread_count)
            .sum::<usize>();
        let unread = if unread > 0 {
            format!(" +{unread}")
        } else {
            String::new()
        };
        Some(format!(
            "team {}:{}{}",
            role_label(member.role),
            member.name,
            unread
        ))
    }

    fn cycle_focus(&mut self, delta: isize) -> bool {
        if self.members.len() < 2 {
            return false;
        }
        if let Some(member) = self.members.get_mut(self.focused_index) {
            member.unread_count = 0;
        }
        let len = self.members.len() as isize;
        self.focused_index = (self.focused_index as isize + delta).rem_euclid(len) as usize;
        if let Some(member) = self.members.get_mut(self.focused_index) {
            member.unread_count = 0;
        }
        true
    }

    fn rebuild_thread_index(&mut self) {
        self.focused_by_thread = self
            .members
            .iter()
            .map(|member| (member.thread_id.clone(), member.id.clone()))
            .collect();
    }
}

pub(super) fn is_team_focus_next_key(key: KeyEvent) -> bool {
    key.code == KeyCode::Down && key.modifiers.contains(KeyModifiers::SHIFT)
}

pub(super) fn is_team_focus_previous_key(key: KeyEvent) -> bool {
    key.code == KeyCode::Up && key.modifiers.contains(KeyModifiers::SHIFT)
}

fn role_label(role: TeamMemberRole) -> &'static str {
    match role {
        TeamMemberRole::Lead => "lead",
        TeamMemberRole::Teammate => "mate",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roder_api::policy_mode::PolicyMode;

    #[test]
    fn focus_cycles_through_lead_and_teammates() {
        let mut state = team_state();

        assert_eq!(state.focused_member().unwrap().id, "lead");
        assert!(state.focus_next());
        assert_eq!(state.focused_member().unwrap().id, "member-1");
        assert!(state.focus_next());
        assert_eq!(state.focused_member().unwrap().id, "member-2");
        assert!(state.focus_next());
        assert_eq!(state.focused_member().unwrap().id, "lead");
        assert!(state.focus_previous());
        assert_eq!(state.focused_member().unwrap().id, "member-2");
    }

    #[test]
    fn focus_member_selects_by_stable_id() {
        let mut state = team_state();

        assert!(state.focus_member("member-2"));
        assert_eq!(state.focused_member().unwrap().id, "member-2");
        assert!(!state.focus_member("missing"));
        assert_eq!(state.focused_member().unwrap().id, "member-2");
    }

    #[test]
    fn unread_counts_skip_focused_member_and_clear_on_focus() {
        let mut state = team_state();

        state.record_thread_activity("thread-b");
        assert_eq!(state.members[1].unread_count, 1);
        assert_eq!(state.focused_label().unwrap(), "team lead:Lead +1");

        state.focus_next();
        assert_eq!(state.focused_member().unwrap().id, "member-1");
        assert_eq!(state.members[1].unread_count, 0);
        assert_eq!(state.focused_label().unwrap(), "team mate:Builder");
    }

    #[test]
    fn modified_arrow_keys_are_team_focus_keys() {
        assert!(is_team_focus_next_key(KeyEvent::new(
            KeyCode::Down,
            KeyModifiers::SHIFT
        )));
        assert!(is_team_focus_previous_key(KeyEvent::new(
            KeyCode::Up,
            KeyModifiers::SHIFT
        )));
        assert!(!is_team_focus_next_key(KeyEvent::new(
            KeyCode::Down,
            KeyModifiers::NONE
        )));
    }

    #[test]
    fn task_list_toggle_is_stateful() {
        let mut state = team_state();

        assert!(!state.show_task_list());
        state.toggle_task_list();
        assert!(state.show_task_list());
        state.toggle_task_list();
        assert!(!state.show_task_list());
    }

    fn team_state() -> TeamUiState {
        let mut state = TeamUiState::default();
        state.set_team(
            "team-1".to_string(),
            vec![
                member("lead", TeamMemberRole::Lead, "Lead", "thread-a"),
                member("member-1", TeamMemberRole::Teammate, "Builder", "thread-b"),
                member("member-2", TeamMemberRole::Teammate, "Reviewer", "thread-c"),
            ],
        );
        state
    }

    fn member(id: &str, role: TeamMemberRole, name: &str, thread_id: &str) -> TeamMemberDescriptor {
        TeamMemberDescriptor {
            id: id.to_string(),
            role,
            name: name.to_string(),
            thread_id: thread_id.to_string(),
            current_turn_id: None,
            model_provider: None,
            model: None,
            policy_mode: PolicyMode::Default,
            status: TeamMemberStatus::Idle,
            pane_id: None,
        }
    }
}
