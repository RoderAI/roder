use super::*;

use roder_api::teams::AgentTeamDisplayMode;

pub(super) const CODEX_V2_MAX_RESIDENT_TEAM_THREADS: usize = 4;

impl Runtime {
    pub(crate) async fn spawn_team_member_for_caller(
        &self,
        parent_thread_id: &ThreadId,
        parent_turn_id: &TurnId,
        req: TeamMemberStartRequest,
    ) -> anyhow::Result<TeamState> {
        let _spawn_guard = self.agent_team_spawn_lock.lock().await;
        let inherited_selection = if req.model_provider.is_none() && req.model.is_none() {
            Some(
                self.inherited_team_member_selection(parent_thread_id, parent_turn_id)
                    .await?,
            )
        } else {
            None
        };
        let team = match self.list_teams().await.into_iter().find(|team| {
            team.lead_thread_id == *parent_thread_id
                || team
                    .members
                    .iter()
                    .any(|member| member.thread_id == *parent_thread_id)
        }) {
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
        let inherited_model = inherited_selection
            .as_ref()
            .map(ModelSelectionMode::concrete_selection)
            .map(|selection| selection.model);
        let requested_model = req.model.as_deref().or(inherited_model.as_deref());
        let lead_model = self
            .selection_mode_for_thread(&team.lead_thread_id)
            .await?
            .map(|selection| selection.concrete_selection().model);
        let is_codex_v2_team = lead_model
            .as_deref()
            .is_some_and(|model| model_supports_reasoning_effort(model, REASONING_ULTRA))
            || requested_model
                .is_some_and(|model| model_supports_reasoning_effort(model, REASONING_ULTRA))
            || team.members.iter().any(|member| {
                member
                    .model
                    .as_deref()
                    .is_some_and(|model| model_supports_reasoning_effort(model, REASONING_ULTRA))
            });
        let resident_threads = team
            .members
            .iter()
            .filter(|member| {
                member.role == roder_api::teams::TeamMemberRole::Lead
                    || member.status != TeamMemberStatus::Closed
            })
            .count();
        if is_codex_v2_team && resident_threads >= CODEX_V2_MAX_RESIDENT_TEAM_THREADS {
            anyhow::bail!(
                "agent thread limit reached for this Codex V2 team: maximum {CODEX_V2_MAX_RESIDENT_TEAM_THREADS} resident threads (the lead plus 3 non-closed teammates); close a teammate before spawning another"
            );
        }

        self.start_team_member_with_selection(&team.id, req, inherited_selection)
            .await
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
        let cfg = self.config.read().await.clone();
        let concrete_selection = selection_mode
            .as_ref()
            .map(ModelSelectionMode::concrete_selection);
        let thread = self
            .create_thread_with(CreateThreadRequest {
                title: Some(req.name.clone()),
                workspace: self.workspace.display().to_string(),
                workspace_id: None,
                root_id: None,
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
                tool_allowlist: Vec::new(),
                developer_instructions: None,
                external_tools: Vec::new(),
                runner: None,
            })
            .await?;
        let team = self
            .read_team(team_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("unknown team {team_id:?}"))?;
        let member_id = format!("member-{}", team.members.len());
        let descriptor = crate::teams::teammate_member(
            member_id.clone(),
            req.name,
            thread.thread_id.clone(),
            req.model_provider.or(thread.provider),
            req.model.or(thread.model),
            cfg.policy_mode,
        );
        let mut next = team;
        next.members.push(descriptor.clone());
        next.updated_at = OffsetDateTime::now_utc();
        let next = self.teams.insert(next).await?;
        self.emit(RoderEvent::TeamMemberStarted(TeamMemberStarted {
            team_id: next.id.clone(),
            member_id,
            member_thread_id: descriptor.thread_id,
            role: descriptor.role,
            name: descriptor.name,
            timestamp: OffsetDateTime::now_utc(),
        }))
        .await;
        Ok(next)
    }
}
