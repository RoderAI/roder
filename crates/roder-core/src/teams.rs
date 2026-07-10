use std::collections::HashMap;
use std::path::PathBuf;

use roder_api::events::{ThreadId, TurnId};
use roder_api::policy_mode::PolicyMode;
use roder_api::teams::{
    AgentTeamDisplayMode, TeamId, TeamMailboxMessage, TeamMailboxMessageKind, TeamMemberDescriptor,
    TeamMemberId, TeamMemberRole, TeamMemberStatus, TeamTaskDescriptor,
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub struct TeamStartRequest {
    pub lead_thread_id: Option<ThreadId>,
    pub display_mode: AgentTeamDisplayMode,
    pub members: Vec<TeamMemberStartRequest>,
}

#[derive(Debug, Clone)]
pub struct TeamMemberStartRequest {
    pub name: String,
    pub model_provider: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TeamState {
    pub id: TeamId,
    pub lead_thread_id: ThreadId,
    pub display_mode: AgentTeamDisplayMode,
    pub members: Vec<TeamMemberDescriptor>,
    pub mailbox: Vec<TeamMailboxMessage>,
    pub tasks: Vec<TeamTaskDescriptor>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

#[derive(Debug)]
pub struct TeamManager {
    teams: RwLock<HashMap<TeamId, TeamState>>,
    mailbox_reservations: RwLock<HashMap<String, TurnId>>,
    data_dir: PathBuf,
}

impl Default for TeamManager {
    fn default() -> Self {
        Self::new(default_team_data_dir())
    }
}

impl TeamManager {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            teams: RwLock::new(HashMap::new()),
            mailbox_reservations: RwLock::new(HashMap::new()),
            data_dir,
        }
    }

    pub async fn insert(&self, team: TeamState) -> anyhow::Result<TeamState> {
        let mut teams = self.teams.write().await;
        self.persist(&team).await?;
        teams.insert(team.id.clone(), team.clone());
        Ok(team)
    }

    pub async fn get(&self, team_id: &str) -> Option<TeamState> {
        if let Some(team) = self.teams.read().await.get(team_id).cloned() {
            return Some(team);
        }
        let mut teams = self.teams.write().await;
        self.load_locked(&mut teams, team_id).await.ok().flatten()
    }

    pub async fn list(&self) -> Vec<TeamState> {
        let mut teams = self.teams.write().await;
        let _ = self.load_all_locked(&mut teams).await;
        let mut listed = teams.values().cloned().collect::<Vec<_>>();
        listed.sort_by_key(|team| std::cmp::Reverse(team.updated_at));
        listed
    }

    pub async fn update_member(
        &self,
        team_id: &str,
        member_id: &str,
        update: impl FnOnce(&mut TeamMemberDescriptor),
    ) -> anyhow::Result<TeamState> {
        let mut teams = self.teams.write().await;
        let mut team = self
            .load_locked(&mut teams, team_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("unknown team {team_id:?}"))?;
        let member = team
            .members
            .iter_mut()
            .find(|member| member.id == member_id)
            .ok_or_else(|| anyhow::anyhow!("unknown team member {member_id:?}"))?;
        update(member);
        team.updated_at = OffsetDateTime::now_utc();
        self.persist(&team).await?;
        teams.insert(team.id.clone(), team.clone());
        Ok(team)
    }

    pub async fn add_member(
        &self,
        team_id: &str,
        member: TeamMemberDescriptor,
    ) -> anyhow::Result<TeamState> {
        let mut teams = self.teams.write().await;
        let mut team = self
            .load_locked(&mut teams, team_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("unknown team {team_id:?}"))?;
        anyhow::ensure!(
            !team.members.iter().any(|existing| existing.id == member.id),
            "team member id {:?} already exists",
            member.id
        );
        if let Some(agent_path) = member.agent_path.as_deref() {
            anyhow::ensure!(
                !team
                    .members
                    .iter()
                    .any(|existing| existing.agent_path.as_deref() == Some(agent_path)),
                "agent path {agent_path:?} already exists"
            );
        }
        team.members.push(member);
        team.updated_at = OffsetDateTime::now_utc();
        self.persist(&team).await?;
        teams.insert(team.id.clone(), team.clone());
        Ok(team)
    }

    pub async fn append_mailbox_message(
        &self,
        team_id: &str,
        from_member_id: Option<TeamMemberId>,
        to_member_id: TeamMemberId,
        kind: TeamMailboxMessageKind,
        text: String,
    ) -> anyhow::Result<TeamState> {
        let mut teams = self.teams.write().await;
        let mut team = self
            .load_locked(&mut teams, team_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("unknown team {team_id:?}"))?;
        if !team.members.iter().any(|member| member.id == to_member_id) {
            anyhow::bail!("unknown team member {to_member_id:?}");
        }
        team.mailbox.push(TeamMailboxMessage {
            id: uuid::Uuid::new_v4().to_string(),
            team_id: team_id.to_string(),
            from_member_id,
            to_member_id,
            kind,
            text,
            delivered: false,
            timestamp: OffsetDateTime::now_utc(),
        });
        team.updated_at = OffsetDateTime::now_utc();
        self.persist(&team).await?;
        teams.insert(team.id.clone(), team.clone());
        Ok(team)
    }

    /// Reserve currently pending messages for one active turn without marking
    /// them delivered. Reservations are process-local: a crash makes the
    /// messages eligible for redelivery, preserving at-least-once delivery.
    pub async fn reserve_pending_mailbox_messages(
        &self,
        team_id: &str,
        member_id: &str,
        turn_id: &TurnId,
    ) -> anyhow::Result<Vec<TeamMailboxMessage>> {
        let mut teams = self.teams.write().await;
        let team = self
            .load_locked(&mut teams, team_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("unknown team {team_id:?}"))?;
        anyhow::ensure!(
            team.members.iter().any(|member| member.id == member_id),
            "unknown team member {member_id:?}"
        );
        let mut reservations = self.mailbox_reservations.write().await;
        let pending = team
            .mailbox
            .into_iter()
            .filter(|message| {
                message.to_member_id == member_id
                    && !message.delivered
                    && !reservations.contains_key(&message.id)
            })
            .collect::<Vec<_>>();
        for message in &pending {
            reservations.insert(message.id.clone(), turn_id.clone());
        }
        Ok(pending)
    }

    pub async fn release_mailbox_reservations_for_turn(&self, turn_id: &TurnId) {
        self.mailbox_reservations
            .write()
            .await
            .retain(|_, reserved_turn_id| reserved_turn_id != turn_id);
    }

    pub async fn release_mailbox_reservations(&self, turn_id: &TurnId, message_ids: &[String]) {
        let message_ids = message_ids.iter().collect::<std::collections::HashSet<_>>();
        self.mailbox_reservations
            .write()
            .await
            .retain(|message_id, reserved_turn_id| {
                reserved_turn_id != turn_id || !message_ids.contains(message_id)
            });
    }

    pub async fn mark_mailbox_messages_delivered(
        &self,
        team_id: &str,
        turn_id: &TurnId,
        message_ids: &[String],
    ) -> anyhow::Result<()> {
        if message_ids.is_empty() {
            return Ok(());
        }
        let mut teams = self.teams.write().await;
        let mut team = self
            .load_locked(&mut teams, team_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("unknown team {team_id:?}"))?;
        let mut reservations = self.mailbox_reservations.write().await;
        let message_ids = message_ids
            .iter()
            .filter(|message_id| reservations.get(*message_id) == Some(turn_id))
            .collect::<std::collections::HashSet<_>>();
        let mut changed = false;
        for message in &mut team.mailbox {
            if message_ids.contains(&message.id) && !message.delivered {
                message.delivered = true;
                changed = true;
            }
        }
        if changed {
            team.updated_at = OffsetDateTime::now_utc();
            self.persist(&team).await?;
            teams.insert(team.id.clone(), team);
        }
        reservations.retain(|message_id, _| !message_ids.contains(message_id));
        Ok(())
    }

    pub async fn set_member_policy_mode(
        &self,
        team_id: &str,
        member_id: &str,
        policy_mode: PolicyMode,
    ) -> anyhow::Result<TeamState> {
        self.update_member(team_id, member_id, |member| {
            member.policy_mode = policy_mode;
        })
        .await
    }

    pub async fn policy_mode_for_thread(&self, thread_id: &str) -> Option<PolicyMode> {
        self.list()
            .await
            .into_iter()
            .flat_map(|team| team.members.into_iter())
            .find(|member| member.thread_id == thread_id)
            .map(|member| member.policy_mode)
    }

    pub async fn member_for_thread(
        &self,
        thread_id: &str,
    ) -> Option<(TeamId, TeamMemberDescriptor)> {
        self.list().await.into_iter().find_map(|team| {
            let team_id = team.id;
            team.members
                .into_iter()
                .find(|member| member.thread_id == thread_id)
                .map(|member| (team_id, member))
        })
    }

    pub async fn complete_member_turn(
        &self,
        thread_id: &str,
        turn_id: &str,
        status: TeamMemberStatus,
        final_message: Option<String>,
        terminal_error: Option<String>,
    ) -> anyhow::Result<Option<(TeamId, TeamMemberDescriptor)>> {
        let mut teams = self.teams.write().await;
        self.load_all_locked(&mut teams).await?;
        let Some(team_id) = teams.iter().find_map(|(team_id, team)| {
            team.members
                .iter()
                .any(|member| {
                    member.thread_id == thread_id
                        && member.current_turn_id.as_deref() == Some(turn_id)
                })
                .then(|| team_id.clone())
        }) else {
            return Ok(None);
        };
        let mut team = teams
            .get(&team_id)
            .cloned()
            .expect("team found by member turn");
        let member = team
            .members
            .iter_mut()
            .find(|member| {
                member.thread_id == thread_id && member.current_turn_id.as_deref() == Some(turn_id)
            })
            .expect("member located by team_for_member_turn");
        member.status = status;
        member.current_turn_id = None;
        member.final_message = final_message;
        member.terminal_error = terminal_error;
        let completed = member.clone();
        team.updated_at = OffsetDateTime::now_utc();
        self.persist(&team).await?;
        teams.insert(team_id.clone(), team);
        Ok(Some((team_id, completed)))
    }

    pub async fn remove(&self, team_id: &str) -> anyhow::Result<Option<TeamState>> {
        let mut teams = self.teams.write().await;
        let _ = self.load_locked(&mut teams, team_id).await?;
        let path = self.team_file(team_id);
        if tokio::fs::try_exists(&path).await.unwrap_or(false) {
            tokio::fs::remove_file(path).await?;
        }
        Ok(teams.remove(team_id))
    }

    async fn persist(&self, team: &TeamState) -> anyhow::Result<()> {
        tokio::fs::create_dir_all(&self.data_dir).await?;
        let data = serde_json::to_vec_pretty(team)?;
        let temp_path = self
            .data_dir
            .join(format!(".team-{}.tmp", uuid::Uuid::new_v4()));
        tokio::fs::write(&temp_path, data).await?;
        if let Err(err) = tokio::fs::rename(&temp_path, self.team_file(&team.id)).await {
            let _ = tokio::fs::remove_file(&temp_path).await;
            return Err(err.into());
        }
        Ok(())
    }

    async fn load_locked(
        &self,
        teams: &mut HashMap<TeamId, TeamState>,
        team_id: &str,
    ) -> anyhow::Result<Option<TeamState>> {
        if let Some(team) = teams.get(team_id).cloned() {
            return Ok(Some(team));
        }
        let path = self.team_file(team_id);
        if !tokio::fs::try_exists(&path).await.unwrap_or(false) {
            return Ok(None);
        }
        let data = tokio::fs::read(path).await?;
        let team = serde_json::from_slice::<TeamState>(&data)?;
        anyhow::ensure!(
            team.id == team_id,
            "persisted team id {:?} does not match file id {team_id:?}",
            team.id
        );
        teams.insert(team.id.clone(), team.clone());
        Ok(Some(team))
    }

    async fn load_all_locked(&self, teams: &mut HashMap<TeamId, TeamState>) -> anyhow::Result<()> {
        if !tokio::fs::try_exists(&self.data_dir).await.unwrap_or(false) {
            return Ok(());
        }
        let mut entries = tokio::fs::read_dir(&self.data_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                continue;
            }
            let Ok(data) = tokio::fs::read(&path).await else {
                continue;
            };
            let Ok(team) = serde_json::from_slice::<TeamState>(&data) else {
                continue;
            };
            if teams.contains_key(&team.id) {
                continue;
            }
            teams.insert(team.id.clone(), team);
        }
        Ok(())
    }

    fn team_file(&self, team_id: &str) -> PathBuf {
        self.data_dir.join(format!("{team_id}.json"))
    }
}

pub(crate) fn lead_member(
    thread_id: ThreadId,
    model_provider: Option<String>,
    model: Option<String>,
    policy_mode: PolicyMode,
) -> TeamMemberDescriptor {
    TeamMemberDescriptor {
        id: "lead".to_string(),
        role: TeamMemberRole::Lead,
        name: "Lead".to_string(),
        task_name: Some("root".to_string()),
        agent_path: Some("/root".to_string()),
        thread_id,
        parent_thread_id: None,
        current_turn_id: None,
        model_provider,
        model,
        policy_mode,
        status: TeamMemberStatus::Idle,
        final_message: None,
        terminal_error: None,
        pane_id: None,
    }
}

pub(crate) fn teammate_member(
    id: TeamMemberId,
    name: String,
    thread_id: ThreadId,
    model_provider: Option<String>,
    model: Option<String>,
    policy_mode: PolicyMode,
) -> TeamMemberDescriptor {
    TeamMemberDescriptor {
        id,
        role: TeamMemberRole::Teammate,
        name,
        task_name: None,
        agent_path: None,
        thread_id,
        parent_thread_id: None,
        current_turn_id: None,
        model_provider,
        model,
        policy_mode,
        status: TeamMemberStatus::Idle,
        final_message: None,
        terminal_error: None,
        pane_id: None,
    }
}

pub(crate) fn default_team_data_dir() -> PathBuf {
    std::env::var_os("RODER_DATA_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".roder")))
        .unwrap_or_else(|| PathBuf::from(".roder"))
        .join("teams")
}
