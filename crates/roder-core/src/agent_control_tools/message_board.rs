use std::sync::Arc;

use roder_api::events::ThreadId;
use roder_api::inference::ToolCallCompleted;
use roder_api::teams::AgentTeamDisplayMode;
use roder_api::tools::ToolResult;
use serde_json::{Value, json};

use crate::runtime::Runtime;
use crate::teams::TeamStartRequest;
use crate::teams::message_board::BoardAction;

use super::{control_error, control_ok};

pub(super) const AGENT_MESSAGE_BOARD: &str = "agent_message_board";

pub(super) fn parameters() -> Value {
    json!({
        "type": "object",
        "required": ["action"],
        "additionalProperties": false,
        "properties": {
            "action": {
                "type": "string",
                "enum": ["list_channels", "create_channel", "subscribe", "unsubscribe",
                    "post", "list_posts", "read_post", "claim", "release", "handoff", "complete"]
            },
            "channel": {
                "type": "string",
                "description": "Required to create, subscribe, or unsubscribe. For post, defaults to general. For list_posts, filters by channel."
            },
            "description": { "type": "string", "description": "Optional create_channel description." },
            "text": { "type": "string", "description": "Required for post. Up to 8192 bytes." },
            "claimable": { "type": "boolean", "description": "Set on post to offer work for an agent to claim." },
            "reply_to": { "type": "string", "description": "Optional ID of a board post to reply to in the same channel." },
            "notify": {
                "type": "array", "items": { "type": "string" }, "maxItems": 32,
                "description": "Additional recipients of a post notice. Accepts agent targets from list_agents. Channel subscribers also receive a notice."
            },
            "post_id": { "type": "string", "description": "Post ID for read_post, claim, release, handoff, or complete." },
            "target": { "type": "string", "description": "Required for handoff. Transfers your claim and sends a notice to this agent." },
            "evidence": { "type": "string", "description": "Required for complete. Record the outcome and verification evidence for your claim." },
            "query": { "type": "string", "description": "Optional case-insensitive text search for list_posts." },
            "before": { "type": "string", "description": "Optional next_before cursor from the previous list_posts page." },
            "limit": { "type": "integer", "minimum": 1, "maximum": 25, "description": "Number of previews per page. Defaults to 10." }
        }
    })
}

impl Runtime {
    pub(super) async fn message_board_tool(
        self: &Arc<Self>,
        caller: &ThreadId,
        call: &ToolCallCompleted,
        arguments: Value,
    ) -> ToolResult {
        let action = match serde_json::from_value::<BoardAction>(arguments) {
            Ok(action) => action,
            Err(err) => return control_error(call, "invalid_arguments", err),
        };
        // Once accepted, finish the team snapshot commit even if the calling turn
        // is interrupted while the filesystem is completing the rename.
        let runtime = Arc::clone(self);
        let caller = caller.clone();
        match tokio::spawn(async move { runtime.apply_message_board_action(&caller, action).await })
            .await
        {
            Ok(Ok(value)) => control_ok(call, value.to_string(), value),
            Ok(Err(err)) => control_error(call, "message_board_error", err),
            Err(err) => control_error(call, "message_board_error", err),
        }
    }

    async fn apply_message_board_action(
        self: &Arc<Self>,
        caller: &ThreadId,
        mut action: BoardAction,
    ) -> anyhow::Result<Value> {
        // Serialize lazy root-team creation with spawn_agent, so the first post and children
        // always land in the same team.
        let _guard = self.agent_team_spawn_lock.lock().await;
        self.ensure_execution_authority()?;
        let team = match self.caller_agents(caller).await.into_iter().next() {
            Some(team) => team,
            None => {
                self.start_team(TeamStartRequest {
                    lead_thread_id: Some(caller.clone()),
                    display_mode: AgentTeamDisplayMode::InProcess,
                    members: Vec::new(),
                })
                .await?
            }
        };
        let actor = team
            .members
            .iter()
            .find(|member| member.thread_id == *caller)
            .ok_or_else(|| anyhow::anyhow!("caller is not a member of this team"))?;
        let resolve = |target: &mut String| -> anyhow::Result<()> {
            *target =
                Self::resolve_agent_target_in_teams(caller, target, std::slice::from_ref(&team))?
                    .member
                    .id;
            Ok(())
        };
        match &mut action {
            BoardAction::Post { notify, .. } => {
                anyhow::ensure!(
                    notify.len() <= 32,
                    "notify accepts at most 32 agent targets"
                );
                for target in notify {
                    resolve(target)?;
                }
            }
            BoardAction::Handoff { target, .. } => resolve(target)?,
            _ => {}
        }
        let result = self
            .teams
            .apply_board_action(&team.id, &actor.id, action)
            .await?;
        for recipient in team
            .members
            .iter()
            .filter(|member| result.recipients.contains(&member.id))
        {
            let _ = self
                .try_deliver_team_mailbox_to_active_turn(&team.id, recipient)
                .await;
        }
        Ok(result.value)
    }
}
