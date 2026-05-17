use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use roder_api::events::{EventEnvelope, RoderEvent, TurnId};
use roder_api::inference::InferenceEvent;
use roder_api::teams::{
    TeamChannel, TeamChannelId, TeamId, TeamMember, TeamMemberId, TeamMessage, TeamSnapshot,
};
use time::OffsetDateTime;
use tokio::sync::RwLock;

use crate::runtime::{CreateSessionRequest, Runtime, StartTurnRequest};
use crate::team_routing::{responders_for_channel_message, team_channel_reply_prompt};

const DEFAULT_TEAM_NAME: &str = "Roder Team";
const DEFAULT_MEMBER_STATUS: &str = "idle";
const WORKING_MEMBER_STATUS: &str = "working";
const ERROR_MEMBER_STATUS: &str = "error";
const FINAL_ANSWER_PHASE: &str = "final_answer";

const DEFAULT_ROLES: [&str; 12] = [
    "engineering-lead",
    "pm",
    "frontend",
    "backend",
    "infra",
    "qa",
    "reviewer",
    "security",
    "docs",
    "research",
    "release",
    "ux",
];

const DEFAULT_CHANNELS: [&str; 9] = [
    "general",
    "standup",
    "reviews",
    "debugging",
    "architecture",
    "shipping",
    "research",
    "ideas",
    "random",
];

#[derive(Default)]
pub(crate) struct TeamStore {
    teams: RwLock<HashMap<TeamId, TeamSnapshot>>,
}

#[derive(Debug, Clone, Default)]
pub struct TeamStartRequest {
    pub name: Option<String>,
    pub workspace: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TeamChannelMessageRequest {
    pub team_id: TeamId,
    pub channel_id: TeamChannelId,
    pub text: String,
    pub author_member_id: Option<TeamMemberId>,
    pub project_context: Option<String>,
    pub thread_ts: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TeamChannelMessageOutcome {
    pub team: TeamSnapshot,
    pub message: TeamMessage,
}

#[derive(Debug, Clone)]
pub struct TeamMemberMessageRequest {
    pub team_id: TeamId,
    pub member_id: TeamMemberId,
    pub channel_id: Option<TeamChannelId>,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct TeamMemberMessageOutcome {
    pub team: TeamSnapshot,
    pub member: TeamMember,
    pub message: TeamMessage,
    pub turn_id: Option<TurnId>,
}

#[derive(Debug, Clone)]
pub struct TeamMemberInterruptOutcome {
    pub team: TeamSnapshot,
    pub member: TeamMember,
}

impl Runtime {
    pub async fn list_teams(&self) -> Vec<TeamSnapshot> {
        let mut teams = self
            .teams
            .teams
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        teams.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        teams
    }

    pub async fn start_team(&self, req: TeamStartRequest) -> anyhow::Result<TeamSnapshot> {
        let cfg = self.status().await;
        let provider = req.provider.unwrap_or(cfg.default_provider);
        let model = req.model.unwrap_or(cfg.default_model);
        let workspace = req.workspace.or(cfg.workspace);
        let name = req
            .name
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_TEAM_NAME.to_string());

        let team_session = self
            .create_session_with(CreateSessionRequest {
                title: Some(name.clone()),
                workspace: workspace.clone(),
                provider: Some(provider.clone()),
                model: Some(model.clone()),
            })
            .await?;
        let now = OffsetDateTime::now_utc();
        let team_id = team_session.thread_id.clone();
        let mut members = Vec::with_capacity(DEFAULT_ROLES.len());
        for role in DEFAULT_ROLES {
            let member_session = self
                .create_session_with(CreateSessionRequest {
                    title: Some(format!("{name} / {role}")),
                    workspace: workspace.clone(),
                    provider: Some(provider.clone()),
                    model: Some(model.clone()),
                })
                .await
                .with_context(|| format!("create team member session for {role}"))?;
            members.push(TeamMember {
                id: role.to_string(),
                role: role.to_string(),
                display_name: display_name_for_role(role),
                thread_id: member_session.thread_id,
                provider: provider.clone(),
                model: model.clone(),
                worktree_path: Some(worktree_path_for(&team_id, role)),
                status: DEFAULT_MEMBER_STATUS.to_string(),
            });
        }

        let mut team = TeamSnapshot {
            id: team_id.clone(),
            name,
            thread_id: team_session.thread_id,
            workspace,
            provider,
            model,
            members,
            channels: DEFAULT_CHANNELS
                .into_iter()
                .map(|channel| TeamChannel {
                    id: channel.to_string(),
                    name: channel.to_string(),
                })
                .collect(),
            messages: Vec::new(),
            aggressive_always_on: true,
            scheduler_running: true,
            created_at: now,
            updated_at: now,
        };
        team.messages.push(TeamMessage {
            id: uuid::Uuid::new_v4().to_string(),
            text: format!("{} started.", team.name),
            author_kind: "system".to_string(),
            author_member_id: None,
            target_member_id: None,
            channel_id: Some("general".to_string()),
            project_context: team.workspace.clone(),
            thread_ts: None,
            turn_id: None,
            created_at: now,
        });
        self.teams.teams.write().await.insert(team_id, team.clone());
        Ok(team)
    }

    pub async fn read_team(&self, team_id: &TeamId) -> anyhow::Result<TeamSnapshot> {
        self.teams
            .teams
            .read()
            .await
            .get(team_id)
            .cloned()
            .with_context(|| format!("team not found: {team_id}"))
    }

    pub async fn cleanup_team(&self, team_id: &TeamId) -> anyhow::Result<bool> {
        let removed = self.teams.teams.write().await.remove(team_id).is_some();
        Ok(removed)
    }

    pub async fn append_team_channel_message(
        self: &Arc<Self>,
        req: TeamChannelMessageRequest,
    ) -> anyhow::Result<TeamChannelMessageOutcome> {
        let team_id = req.team_id.clone();
        let channel_id = req.channel_id.clone();
        let text = req.text.clone();
        let (message, responders) = {
            let mut teams = self.teams.teams.write().await;
            let team = teams
                .get_mut(&req.team_id)
                .with_context(|| format!("team not found: {}", req.team_id))?;
            ensure_channel(team, &req.channel_id)?;
            if let Some(member_id) = &req.author_member_id {
                ensure_member(team, member_id)?;
            }
            let responders = if req.author_member_id.is_none() && team.scheduler_running {
                responders_for_channel_message(team, &req.channel_id, &req.text)
            } else {
                Vec::new()
            };
            let message = TeamMessage {
                id: uuid::Uuid::new_v4().to_string(),
                text: req.text,
                author_kind: if req.author_member_id.is_some() {
                    "member".to_string()
                } else {
                    "user".to_string()
                },
                author_member_id: req.author_member_id,
                target_member_id: None,
                channel_id: Some(req.channel_id),
                project_context: req.project_context,
                thread_ts: req.thread_ts,
                turn_id: None,
                created_at: OffsetDateTime::now_utc(),
            };
            team.messages.push(message.clone());
            team.updated_at = OffsetDateTime::now_utc();
            (message, responders)
        };

        for member_id in responders {
            let _ = self
                .start_team_member_channel_turn(
                    team_id.clone(),
                    member_id,
                    channel_id.clone(),
                    text.clone(),
                )
                .await;
        }

        let team = self.read_team(&team_id).await?;
        Ok(TeamChannelMessageOutcome { team, message })
    }

    pub async fn set_team_scheduler(
        &self,
        team_id: &TeamId,
        running: bool,
    ) -> anyhow::Result<TeamSnapshot> {
        let mut teams = self.teams.teams.write().await;
        let team = teams
            .get_mut(team_id)
            .with_context(|| format!("team not found: {team_id}"))?;
        team.scheduler_running = running;
        team.updated_at = OffsetDateTime::now_utc();
        Ok(team.clone())
    }

    pub async fn append_team_member_message(
        self: &Arc<Self>,
        req: TeamMemberMessageRequest,
    ) -> anyhow::Result<TeamMemberMessageOutcome> {
        let turn_events = self.subscribe_events();
        let team_id = req.team_id.clone();
        let member_id = req.member_id.clone();
        let channel_id = req.channel_id.clone();
        let prompt = req.text.clone();
        let (member_thread_id, member_provider, member_model, message_id) = {
            let mut teams = self.teams.teams.write().await;
            let team = teams
                .get_mut(&req.team_id)
                .with_context(|| format!("team not found: {}", req.team_id))?;
            if let Some(channel_id) = &req.channel_id {
                ensure_channel(team, channel_id)?;
            }
            let member = ensure_member(team, &req.member_id)?.clone();
            let message = TeamMessage {
                id: uuid::Uuid::new_v4().to_string(),
                text: req.text.clone(),
                author_kind: "user".to_string(),
                author_member_id: None,
                target_member_id: Some(req.member_id.clone()),
                channel_id: req.channel_id.clone(),
                project_context: team.workspace.clone(),
                thread_ts: None,
                turn_id: None,
                created_at: OffsetDateTime::now_utc(),
            };
            let message_id = message.id.clone();
            team.messages.push(message);
            team.updated_at = OffsetDateTime::now_utc();
            (member.thread_id, member.provider, member.model, message_id)
        };

        let turn_id = self
            .start_turn(StartTurnRequest {
                thread_id: member_thread_id.clone(),
                message: prompt,
                images: Vec::new(),
                provider_override: Some(member_provider),
                model_override: Some(member_model),
                instructions: crate::default_instructions(),
            })
            .await
            .ok();

        if let Some(turn_id) = &turn_id {
            self.spawn_team_member_turn_monitor(
                team_id.clone(),
                member_id.clone(),
                channel_id,
                member_thread_id,
                turn_id.clone(),
                turn_events,
            );
        }

        let mut teams = self.teams.teams.write().await;
        let team = teams
            .get_mut(&team_id)
            .with_context(|| format!("team not found: {team_id}"))?;
        let member = ensure_member_mut(team, &member_id)?;
        if turn_id.is_some() {
            member.status = WORKING_MEMBER_STATUS.to_string();
        }
        let member = member.clone();
        let message = team
            .messages
            .iter_mut()
            .find(|message| message.id == message_id)
            .context("team member message disappeared")?;
        message.turn_id = turn_id.clone();
        let message = message.clone();
        team.updated_at = OffsetDateTime::now_utc();
        Ok(TeamMemberMessageOutcome {
            team: team.clone(),
            member,
            message,
            turn_id,
        })
    }

    async fn start_team_member_channel_turn(
        self: &Arc<Self>,
        team_id: TeamId,
        member_id: TeamMemberId,
        channel_id: TeamChannelId,
        channel_message_text: String,
    ) -> anyhow::Result<Option<TurnId>> {
        let turn_events = self.subscribe_events();
        let (member_thread_id, member_provider, member_model, prompt) = {
            let teams = self.teams.teams.read().await;
            let team = teams
                .get(&team_id)
                .with_context(|| format!("team not found: {team_id}"))?;
            ensure_channel(team, &channel_id)?;
            let member = ensure_member(team, &member_id)?.clone();
            (
                member.thread_id.clone(),
                member.provider.clone(),
                member.model.clone(),
                team_channel_reply_prompt(&channel_id, &channel_message_text, &member),
            )
        };

        let turn_id = self
            .start_turn(StartTurnRequest {
                thread_id: member_thread_id.clone(),
                message: prompt,
                images: Vec::new(),
                provider_override: Some(member_provider),
                model_override: Some(member_model),
                instructions: crate::default_instructions(),
            })
            .await
            .ok();

        if let Some(turn_id) = &turn_id {
            self.set_team_member_status(&team_id, &member_id, WORKING_MEMBER_STATUS)
                .await;
            self.spawn_team_member_turn_monitor(
                team_id,
                member_id,
                Some(channel_id),
                member_thread_id,
                turn_id.clone(),
                turn_events,
            );
        }

        Ok(turn_id)
    }

    fn spawn_team_member_turn_monitor(
        self: &Arc<Self>,
        team_id: TeamId,
        member_id: TeamMemberId,
        channel_id: Option<TeamChannelId>,
        member_thread_id: roder_api::events::ThreadId,
        turn_id: TurnId,
        mut turn_events: tokio::sync::broadcast::Receiver<EventEnvelope>,
    ) {
        let runtime = Arc::clone(self);
        tokio::spawn(async move {
            let mut reply_text = String::new();
            let timeout = runtime.team_member_turn_timeout().await;
            loop {
                match tokio::time::timeout(timeout, turn_events.recv()).await {
                    Err(_) => {
                        let _ = runtime
                            .interrupt_turn(member_thread_id, turn_id.clone())
                            .await;
                        runtime
                            .set_team_member_status(&team_id, &member_id, ERROR_MEMBER_STATUS)
                            .await;
                        break;
                    }
                    Ok(Ok(envelope)) => match envelope.event {
                        RoderEvent::InferenceEventReceived(event) if event.turn_id == turn_id => {
                            if let InferenceEvent::MessageDelta(delta) = event.event
                                && is_final_team_reply_phase(delta.phase.as_deref())
                            {
                                reply_text.push_str(&delta.text);
                            }
                        }
                        RoderEvent::TurnCompleted(event) if event.turn_id == turn_id => {
                            let _ = runtime
                                .complete_team_member_turn(
                                    &team_id, &member_id, channel_id, &turn_id, reply_text,
                                )
                                .await;
                            break;
                        }
                        RoderEvent::TurnFailed(event) if event.turn_id == turn_id => {
                            runtime
                                .set_team_member_status(&team_id, &member_id, ERROR_MEMBER_STATUS)
                                .await;
                            break;
                        }
                        RoderEvent::TurnInterrupted(event) if event.turn_id == turn_id => {
                            runtime
                                .set_team_member_status(&team_id, &member_id, DEFAULT_MEMBER_STATUS)
                                .await;
                            break;
                        }
                        _ => {}
                    },
                    Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
                    Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => break,
                }
            }
        });
    }

    async fn complete_team_member_turn(
        &self,
        team_id: &TeamId,
        member_id: &TeamMemberId,
        channel_id: Option<TeamChannelId>,
        turn_id: &TurnId,
        reply_text: String,
    ) -> anyhow::Result<()> {
        let mut teams = self.teams.teams.write().await;
        let team = teams
            .get_mut(team_id)
            .with_context(|| format!("team not found: {team_id}"))?;
        let now = OffsetDateTime::now_utc();
        let text = reply_text.trim().to_string();
        if !text.is_empty() {
            team.messages.push(TeamMessage {
                id: uuid::Uuid::new_v4().to_string(),
                text,
                author_kind: "member".to_string(),
                author_member_id: Some(member_id.clone()),
                target_member_id: None,
                channel_id,
                project_context: team.workspace.clone(),
                thread_ts: None,
                turn_id: Some(turn_id.clone()),
                created_at: now,
            });
        }
        if let Some(member) = team
            .members
            .iter_mut()
            .find(|member| member.id == *member_id)
        {
            member.status = DEFAULT_MEMBER_STATUS.to_string();
        }
        team.updated_at = now;
        Ok(())
    }

    async fn set_team_member_status(
        &self,
        team_id: &TeamId,
        member_id: &TeamMemberId,
        status: &str,
    ) {
        let mut teams = self.teams.teams.write().await;
        let Some(team) = teams.get_mut(team_id) else {
            return;
        };
        let Some(member) = team
            .members
            .iter_mut()
            .find(|member| member.id == *member_id)
        else {
            return;
        };
        member.status = status.to_string();
        team.updated_at = OffsetDateTime::now_utc();
    }

    pub async fn interrupt_team_member(
        &self,
        team_id: &TeamId,
        member_id: &TeamMemberId,
    ) -> anyhow::Result<TeamMemberInterruptOutcome> {
        let (thread_id, turn_id) = {
            let teams = self.teams.teams.read().await;
            let team = teams
                .get(team_id)
                .with_context(|| format!("team not found: {team_id}"))?;
            let member = ensure_member(team, member_id)?;
            let turn_id = team
                .messages
                .iter()
                .rev()
                .find(|message| {
                    message.author_member_id.as_deref() == Some(member_id.as_str())
                        || message.target_member_id.as_deref() == Some(member_id.as_str())
                })
                .and_then(|message| message.turn_id.clone());
            (member.thread_id.clone(), turn_id)
        };

        if let Some(turn_id) = turn_id {
            let _ = self.interrupt_turn(thread_id, turn_id).await;
        }

        let mut teams = self.teams.teams.write().await;
        let team = teams
            .get_mut(team_id)
            .with_context(|| format!("team not found: {team_id}"))?;
        let member = ensure_member_mut(team, member_id)?;
        member.status = DEFAULT_MEMBER_STATUS.to_string();
        let member = member.clone();
        team.updated_at = OffsetDateTime::now_utc();
        Ok(TeamMemberInterruptOutcome {
            team: team.clone(),
            member,
        })
    }
}

fn ensure_channel(team: &TeamSnapshot, channel_id: &str) -> anyhow::Result<()> {
    if team.channels.iter().any(|channel| channel.id == channel_id) {
        return Ok(());
    }
    anyhow::bail!("team channel not found: {channel_id}");
}

fn ensure_member<'a>(team: &'a TeamSnapshot, member_id: &str) -> anyhow::Result<&'a TeamMember> {
    team.members
        .iter()
        .find(|member| member.id == member_id)
        .with_context(|| format!("team member not found: {member_id}"))
}

fn ensure_member_mut<'a>(
    team: &'a mut TeamSnapshot,
    member_id: &str,
) -> anyhow::Result<&'a mut TeamMember> {
    team.members
        .iter_mut()
        .find(|member| member.id == member_id)
        .with_context(|| format!("team member not found: {member_id}"))
}

fn display_name_for_role(role: &str) -> String {
    role.split('-')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn worktree_path_for(team_id: &str, role: &str) -> String {
    let mut path = PathBuf::from(std::env::temp_dir());
    path.push("roder");
    path.push("teams");
    path.push(team_id);
    path.push(role);
    path.to_string_lossy().to_string()
}

fn is_final_team_reply_phase(phase: Option<&str>) -> bool {
    phase.is_none_or(|phase| phase.is_empty() || phase == FINAL_ANSWER_PHASE)
}
