use std::collections::HashMap;
use std::path::PathBuf;

use roder_api::events::ThreadId;
use roder_api::policy_mode::PolicyMode;
use roder_api::teams::{
    AgentTeamDisplayMode, TeamId, TeamMailboxMessage, TeamMemberDescriptor, TeamMemberId,
    TeamMemberRole, TeamMemberStatus, TeamTaskDescriptor,
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
            data_dir,
        }
    }

    pub async fn insert(&self, team: TeamState) -> anyhow::Result<TeamState> {
        self.persist(&team).await?;
        self.teams
            .write()
            .await
            .insert(team.id.clone(), team.clone());
        Ok(team)
    }

    pub async fn get(&self, team_id: &str) -> Option<TeamState> {
        if let Some(team) = self.teams.read().await.get(team_id).cloned() {
            return Some(team);
        }
        self.load(team_id).await.ok().flatten()
    }

    pub async fn list(&self) -> Vec<TeamState> {
        let mut teams = self
            .teams
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        teams.sort_by_key(|team| std::cmp::Reverse(team.updated_at));
        teams
    }

    pub async fn update_member(
        &self,
        team_id: &str,
        member_id: &str,
        update: impl FnOnce(&mut TeamMemberDescriptor),
    ) -> anyhow::Result<TeamState> {
        let mut team = self
            .get(team_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("unknown team {team_id:?}"))?;
        let member = team
            .members
            .iter_mut()
            .find(|member| member.id == member_id)
            .ok_or_else(|| anyhow::anyhow!("unknown team member {member_id:?}"))?;
        update(member);
        team.updated_at = OffsetDateTime::now_utc();
        self.insert(team).await
    }

    pub async fn append_mailbox_message(
        &self,
        team_id: &str,
        from_member_id: Option<TeamMemberId>,
        to_member_id: TeamMemberId,
        text: String,
    ) -> anyhow::Result<TeamState> {
        let mut team = self
            .get(team_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("unknown team {team_id:?}"))?;
        if !team.members.iter().any(|member| member.id == to_member_id) {
            anyhow::bail!("unknown team member {to_member_id:?}");
        }
        team.mailbox.push(TeamMailboxMessage {
            id: uuid::Uuid::new_v4().to_string(),
            team_id: team_id.to_string(),
            from_member_id,
            to_member_id,
            text,
            timestamp: OffsetDateTime::now_utc(),
        });
        team.updated_at = OffsetDateTime::now_utc();
        self.insert(team).await
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
        self.teams
            .read()
            .await
            .values()
            .flat_map(|team| team.members.iter())
            .find(|member| member.thread_id == thread_id)
            .map(|member| member.policy_mode)
    }

    pub async fn member_for_thread(
        &self,
        thread_id: &str,
    ) -> Option<(TeamId, TeamMemberDescriptor)> {
        self.teams.read().await.values().find_map(|team| {
            team.members
                .iter()
                .find(|member| member.thread_id == thread_id)
                .cloned()
                .map(|member| (team.id.clone(), member))
        })
    }

    pub async fn complete_member_turn(
        &self,
        thread_id: &str,
        turn_id: &str,
        status: TeamMemberStatus,
    ) -> anyhow::Result<Option<(TeamId, TeamMemberDescriptor)>> {
        let Some(mut team) = self.team_for_member_turn(thread_id, turn_id).await else {
            return Ok(None);
        };
        let team_id = team.id.clone();
        let member = team
            .members
            .iter_mut()
            .find(|member| {
                member.thread_id == thread_id && member.current_turn_id.as_deref() == Some(turn_id)
            })
            .expect("member located by team_for_member_turn");
        member.status = status;
        member.current_turn_id = None;
        let completed = member.clone();
        team.updated_at = OffsetDateTime::now_utc();
        self.insert(team).await?;
        Ok(Some((team_id, completed)))
    }

    pub async fn remove(&self, team_id: &str) -> anyhow::Result<Option<TeamState>> {
        let removed = self.teams.write().await.remove(team_id);
        let path = self.team_file(team_id);
        if tokio::fs::try_exists(&path).await.unwrap_or(false) {
            tokio::fs::remove_file(path).await?;
        }
        Ok(removed)
    }

    async fn persist(&self, team: &TeamState) -> anyhow::Result<()> {
        tokio::fs::create_dir_all(&self.data_dir).await?;
        let data = serde_json::to_vec_pretty(team)?;
        tokio::fs::write(self.team_file(&team.id), data).await?;
        Ok(())
    }

    async fn load(&self, team_id: &str) -> anyhow::Result<Option<TeamState>> {
        let path = self.team_file(team_id);
        if !tokio::fs::try_exists(&path).await.unwrap_or(false) {
            return Ok(None);
        }
        let data = tokio::fs::read(path).await?;
        let team = serde_json::from_slice::<TeamState>(&data)?;
        self.teams
            .write()
            .await
            .insert(team.id.clone(), team.clone());
        Ok(Some(team))
    }

    async fn team_for_member_turn(&self, thread_id: &str, turn_id: &str) -> Option<TeamState> {
        self.teams
            .read()
            .await
            .values()
            .find(|team| {
                team.members.iter().any(|member| {
                    member.thread_id == thread_id
                        && member.current_turn_id.as_deref() == Some(turn_id)
                })
            })
            .cloned()
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
        thread_id,
        current_turn_id: None,
        model_provider,
        model,
        policy_mode,
        status: TeamMemberStatus::Idle,
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
        thread_id,
        current_turn_id: None,
        model_provider,
        model,
        policy_mode,
        status: TeamMemberStatus::Idle,
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
