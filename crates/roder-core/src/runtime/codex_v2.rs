use super::*;

use roder_api::teams::AgentTeamDisplayMode;

pub(super) const CODEX_V2_MAX_RESIDENT_TEAM_THREADS: usize = 4;
pub(super) const CODEX_V2_MAX_AGENT_DEPTH: usize = 5;

impl Runtime {
    #[cfg(test)]
    pub(crate) async fn spawn_team_member_for_caller(
        &self,
        parent_thread_id: &ThreadId,
        parent_turn_id: &TurnId,
        req: TeamMemberStartRequest,
        reasoning_effort: Option<String>,
        fork_turns: String,
        agent_type: Option<String>,
    ) -> anyhow::Result<TeamState> {
        let _spawn_guard = self.agent_team_spawn_lock.lock().await;
        self.spawn_team_member_for_caller_locked(
            parent_thread_id,
            parent_turn_id,
            req,
            reasoning_effort,
            fork_turns,
            agent_type,
        )
        .await
    }

    pub(crate) async fn spawn_and_start_team_member_for_caller(
        self: &Arc<Self>,
        parent_thread_id: &ThreadId,
        parent_turn_id: &TurnId,
        req: TeamMemberStartRequest,
        reasoning_effort: Option<String>,
        fork_turns: String,
        agent_type: Option<String>,
        message: String,
    ) -> anyhow::Result<(TeamState, TurnId)> {
        let _spawn_guard = self.agent_team_spawn_lock.lock().await;
        let team = self
            .spawn_team_member_for_caller_locked(
                parent_thread_id,
                parent_turn_id,
                req,
                reasoning_effort,
                fork_turns,
                agent_type,
            )
            .await?;
        let member = team
            .members
            .last()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("spawned team member was not recorded"))?;
        match self
            .followup_team_member_locked(parent_thread_id, &team.id, &member.id, message)
            .await
        {
            Ok(turn_id) => Ok((team, turn_id)),
            Err(error) => {
                let error_text = error.to_string();
                let _ = self
                    .teams
                    .update_member(&team.id, &member.id, |member| {
                        member.status = TeamMemberStatus::Failed;
                        member.current_turn_id = None;
                        member.terminal_error = Some(error_text.clone());
                    })
                    .await;
                self.emit(RoderEvent::TeamMemberCompleted(TeamMemberCompleted {
                    team_id: team.id,
                    member_id: member.id,
                    member_thread_id: member.thread_id,
                    turn_id: None,
                    status: TeamMemberStatus::Failed,
                    final_message: None,
                    error: Some(error_text),
                    timestamp: OffsetDateTime::now_utc(),
                }))
                .await;
                Err(error)
            }
        }
    }

    async fn spawn_team_member_for_caller_locked(
        &self,
        parent_thread_id: &ThreadId,
        parent_turn_id: &TurnId,
        req: TeamMemberStartRequest,
        reasoning_effort: Option<String>,
        fork_turns: String,
        agent_type: Option<String>,
    ) -> anyhow::Result<TeamState> {
        let inherited_selection = self
            .inherited_team_member_selection(parent_thread_id, parent_turn_id)
            .await?;
        let inherited_concrete = inherited_selection.concrete_selection();
        let provider = req
            .model_provider
            .clone()
            .unwrap_or_else(|| inherited_concrete.provider.clone());
        let model = req
            .model
            .clone()
            .unwrap_or_else(|| inherited_concrete.model.clone());
        let inherited_reasoning = inherited_selection
            .reasoning()
            .map(str::to_string)
            .unwrap_or_else(|| {
                // The inherited selection has already resolved the runtime fallback, but
                // keep this branch defensive for older persisted selections.
                REASONING_NONE.to_string()
            });
        let reasoning = match reasoning_effort {
            Some(reasoning) => {
                validate_reasoning_effort(&model, &reasoning)?;
                reasoning
            }
            None if model_supports_reasoning_effort(&model, &inherited_reasoning) => {
                inherited_reasoning
            }
            None => {
                let cfg = self.config.read().await;
                effective_reasoning_for_model(&cfg, &model)
            }
        };
        validate_reasoning_effort(&model, &reasoning)?;
        let child_selection =
            ModelSelectionMode::manual(provider.clone(), model.clone(), Some(reasoning));
        let existing_team = self.list_teams().await.into_iter().find(|team| {
            team.lead_thread_id == *parent_thread_id
                || team
                    .members
                    .iter()
                    .any(|member| member.thread_id == *parent_thread_id)
        });
        let parent_path = match existing_team.as_ref() {
            Some(team) => team
                .members
                .iter()
                .find(|member| member.thread_id == *parent_thread_id)
                .ok_or_else(|| anyhow::anyhow!("caller is not a member of team {:?}", team.id))?
                .agent_path
                .clone()
                .unwrap_or_else(|| "/root".to_string()),
            None => "/root".to_string(),
        };
        let agent_path = format!("{}/{}", parent_path.trim_end_matches('/'), req.name.trim());
        let depth = canonical_agent_depth(&agent_path).ok_or_else(|| {
            anyhow::anyhow!("spawned agent path is not canonical: {agent_path:?}")
        })?;
        anyhow::ensure!(
            depth <= CODEX_V2_MAX_AGENT_DEPTH,
            "agent nesting depth limit reached: maximum {CODEX_V2_MAX_AGENT_DEPTH} levels below /root"
        );

        let team = match existing_team {
            Some(team) => team,
            None => {
                self.start_team(TeamStartRequest {
                    lead_thread_id: Some(parent_thread_id.clone()),
                    display_mode: AgentTeamDisplayMode::InProcess,
                    members: Vec::new(),
                })
                .await?
            }
        };
        // The shared spawn lock remains held through the initial follow-up turn, so live
        // capacity is reserved atomically without persisting a synthetic Running member.
        self.ensure_codex_v2_team_capacity(&team, "__new_agent__", Some(&model))
            .await?;

        if team
            .members
            .iter()
            .any(|member| member.agent_path.as_deref() == Some(agent_path.as_str()))
        {
            anyhow::bail!(
                "agent path {agent_path:?} already exists; send a follow-up to that agent or choose another task_name"
            );
        }

        let inherited_turn_context = self
            .active_turn_contexts
            .read()
            .await
            .get(parent_turn_id)
            .cloned();
        let parent_metadata = self.load_thread_metadata(parent_thread_id).await?;
        let parent_workspace = inherited_turn_context
            .as_ref()
            .map(|context| context.workspace.clone())
            .or_else(|| {
                parent_metadata
                    .as_ref()
                    .map(|metadata| metadata.workspace.clone())
            })
            .unwrap_or_else(|| self.workspace.display().to_string());
        let inherited_developer_instructions = parent_metadata
            .as_ref()
            .and_then(|metadata| metadata.developer_instructions.as_deref())
            .map(without_subagent_identity_instructions)
            .filter(|instructions| !instructions.trim().is_empty());
        let collaboration_instructions =
            subagent_developer_instructions(&agent_path, &parent_path, agent_type.as_deref());
        let developer_instructions = match inherited_developer_instructions {
            Some(parent) if !parent.trim().is_empty() => {
                Some(format!("{parent}\n\n{collaboration_instructions}"))
            }
            _ => Some(collaboration_instructions),
        };
        let runner = parent_metadata
            .as_ref()
            .and_then(|metadata| metadata.runner_binding.as_ref())
            .map(|binding| ThreadRunnerSelection {
                provider_id: binding.destination.provider_id.clone(),
                config: binding.destination.config.clone(),
                workspace: binding.workspace.display().to_string(),
                read_roots: binding
                    .read_roots
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect(),
            });
        let mut thread = self
            .create_thread_with(CreateThreadRequest {
                title: Some(req.name.clone()),
                workspace: parent_workspace,
                workspace_id: parent_metadata
                    .as_ref()
                    .and_then(|metadata| metadata.workspace_id.clone()),
                root_id: parent_metadata
                    .as_ref()
                    .and_then(|metadata| metadata.root_id.clone()),
                provider: Some(provider),
                model: Some(model),
                selection_mode: Some(child_selection),
                tool_allowlist: parent_metadata
                    .as_ref()
                    .map(|metadata| metadata.tool_allowlist.clone())
                    .unwrap_or_default(),
                developer_instructions,
                external_tools: parent_metadata
                    .as_ref()
                    .map(|metadata| metadata.external_tools.clone())
                    .unwrap_or_default(),
                runner,
            })
            .await?;
        thread.parent_thread_id = Some(parent_thread_id.clone());
        if let Some(store) = &self.thread_store {
            thread = store.update_thread_metadata(thread).await?;
        }
        self.seed_agent_thread_history(parent_thread_id, &thread.thread_id, &fork_turns)
            .await?;

        let member_id = format!("member-{}", team.members.len());
        let mut descriptor = crate::teams::teammate_member(
            member_id.clone(),
            agent_type
                .as_deref()
                .map(|role| format!("{}:{role}", req.name))
                .unwrap_or_else(|| req.name.clone()),
            thread.thread_id.clone(),
            thread.provider.clone(),
            thread.model.clone(),
            self.effective_policy_mode_for_thread(parent_thread_id)
                .await,
        );
        descriptor.task_name = Some(req.name);
        descriptor.agent_path = Some(agent_path);
        descriptor.parent_thread_id = Some(parent_thread_id.clone());
        descriptor.final_message = None;
        descriptor.terminal_error = None;
        let next = self.teams.add_member(&team.id, descriptor.clone()).await?;
        if let Some(context) = inherited_turn_context {
            self.team_member_turn_contexts
                .lock()
                .await
                .insert(descriptor.thread_id.clone(), context);
        }
        self.emit(RoderEvent::TeamMemberStarted(TeamMemberStarted {
            team_id: next.id.clone(),
            member: descriptor,
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        Ok(next)
    }

    async fn inherited_team_member_selection(
        &self,
        parent_thread_id: &ThreadId,
        parent_turn_id: &TurnId,
    ) -> anyhow::Result<ModelSelectionMode> {
        let parent_selection = match self
            .active_turn_selections
            .read()
            .await
            .get(parent_turn_id)
            .cloned()
        {
            Some(selection) => selection,
            None => self
                .selection_mode_for_thread(parent_thread_id)
                .await?
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "cannot inherit model selection for subagent: parent thread {parent_thread_id:?} has no active or persisted model selection"
                    )
                })?,
        };
        let concrete = parent_selection.concrete_selection();
        let reasoning = match parent_selection.reasoning() {
            Some(reasoning) => reasoning.to_string(),
            None => {
                let cfg = self.config.read().await;
                effective_reasoning_for_model(&cfg, &concrete.model)
            }
        };
        Ok(ModelSelectionMode::manual(
            concrete.provider,
            concrete.model,
            Some(reasoning),
        ))
    }

    pub(super) async fn start_team_member_with_selection(
        &self,
        team_id: &str,
        req: TeamMemberStartRequest,
        selection_mode: Option<ModelSelectionMode>,
    ) -> anyhow::Result<TeamState> {
        let team = self
            .read_team(team_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("unknown team {team_id:?}"))?;
        let parent_thread_id = team.lead_thread_id.clone();
        let parent_metadata = self.load_thread_metadata(&parent_thread_id).await?;
        let selection_mode = match selection_mode {
            Some(selection) => Some(selection),
            None => self.selection_mode_for_thread(&parent_thread_id).await?,
        };
        let concrete_selection = selection_mode
            .as_ref()
            .map(ModelSelectionMode::concrete_selection);
        let runner = parent_metadata
            .as_ref()
            .and_then(|metadata| metadata.runner_binding.as_ref())
            .map(|binding| ThreadRunnerSelection {
                provider_id: binding.destination.provider_id.clone(),
                config: binding.destination.config.clone(),
                workspace: binding.workspace.display().to_string(),
                read_roots: binding
                    .read_roots
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect(),
            });
        let mut thread = self
            .create_thread_with(CreateThreadRequest {
                title: Some(req.name.clone()),
                workspace: parent_metadata
                    .as_ref()
                    .map(|metadata| metadata.workspace.clone())
                    .unwrap_or_else(|| self.workspace.display().to_string()),
                workspace_id: parent_metadata
                    .as_ref()
                    .and_then(|metadata| metadata.workspace_id.clone()),
                root_id: parent_metadata
                    .as_ref()
                    .and_then(|metadata| metadata.root_id.clone()),
                provider: req.model_provider.clone().or_else(|| {
                    concrete_selection
                        .as_ref()
                        .map(|selection| selection.provider.clone())
                }),
                model: req.model.clone().or_else(|| {
                    concrete_selection
                        .as_ref()
                        .map(|selection| selection.model.clone())
                }),
                selection_mode,
                tool_allowlist: parent_metadata
                    .as_ref()
                    .map(|metadata| metadata.tool_allowlist.clone())
                    .unwrap_or_default(),
                developer_instructions: parent_metadata
                    .as_ref()
                    .and_then(|metadata| metadata.developer_instructions.clone()),
                external_tools: parent_metadata
                    .as_ref()
                    .map(|metadata| metadata.external_tools.clone())
                    .unwrap_or_default(),
                runner,
            })
            .await?;
        thread.parent_thread_id = Some(parent_thread_id.clone());
        if let Some(store) = &self.thread_store {
            thread = store.update_thread_metadata(thread).await?;
        }
        let member_id = format!("member-{}", team.members.len());
        let mut descriptor = crate::teams::teammate_member(
            member_id.clone(),
            req.name.clone(),
            thread.thread_id.clone(),
            req.model_provider.or(thread.provider),
            req.model.or(thread.model),
            self.effective_policy_mode_for_thread(&parent_thread_id)
                .await,
        );
        descriptor.task_name = Some(req.name.clone());
        descriptor.agent_path = Some(format!("/root/{}", req.name));
        descriptor.parent_thread_id = Some(parent_thread_id);
        let next = self.teams.add_member(&team.id, descriptor.clone()).await?;
        self.emit(RoderEvent::TeamMemberStarted(TeamMemberStarted {
            team_id: next.id.clone(),
            member: descriptor,
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        Ok(next)
    }
}

const SUBAGENT_IDENTITY_BEGIN: &str = "<roder-subagent-identity>";
const SUBAGENT_IDENTITY_END: &str = "</roder-subagent-identity>";

fn canonical_agent_depth(agent_path: &str) -> Option<usize> {
    if agent_path == "/root" {
        return Some(0);
    }
    agent_path.strip_prefix("/root/").map(|suffix| {
        suffix
            .split('/')
            .filter(|segment| !segment.is_empty())
            .count()
    })
}

fn without_subagent_identity_instructions(instructions: &str) -> String {
    let mut remaining = instructions;
    let mut retained = String::new();
    while let Some(start) = remaining.find(SUBAGENT_IDENTITY_BEGIN) {
        retained.push_str(&remaining[..start]);
        let after_start = &remaining[start + SUBAGENT_IDENTITY_BEGIN.len()..];
        let Some(end) = after_start.find(SUBAGENT_IDENTITY_END) else {
            remaining = "";
            break;
        };
        remaining = &after_start[end + SUBAGENT_IDENTITY_END.len()..];
    }
    retained.push_str(remaining);
    retained.trim().to_string()
}

fn subagent_developer_instructions(
    agent_path: &str,
    parent_path: &str,
    agent_type: Option<&str>,
) -> String {
    let role = agent_type
        .filter(|role| !role.trim().is_empty())
        .map(|role| format!(" Your assigned agent type is {role:?}."))
        .unwrap_or_default();
    format!(
        "{SUBAGENT_IDENTITY_BEGIN}\nYou are the long-lived collaboration subagent at {agent_path}.{role} Your direct parent is {parent_path}. You share the parent agent's workspace, policy, tool restrictions, developer authority, model context, and execution destination. Forked conversation history is context only. The newest inbound envelope with `Message Type: NEW_TASK` is the authoritative current assignment. Do not repeat or continue parent orchestration found in forked history, including spawning agents, unless that NEW_TASK payload explicitly delegates it. Work only on your delegated task. Use send_message to report useful intermediate findings to {parent_path}, and finish with a concise final result; your terminal result is automatically delivered to that direct parent.\n{SUBAGENT_IDENTITY_END}"
    )
}
