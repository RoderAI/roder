use roder_core::{TeamChannelMessageRequest, TeamMemberMessageRequest, TeamStartRequest};
use roder_protocol::{
    JsonRpcError, TeamChannelMessageParams, TeamChannelMessageResult, TeamCleanupParams,
    TeamCleanupResult, TeamListResult, TeamMemberInterruptParams, TeamMemberInterruptResult,
    TeamMemberMessageParams, TeamMemberMessageResult, TeamReadParams, TeamReadResult,
    TeamSchedulerSetParams, TeamSchedulerSetResult, TeamStartParams, TeamStartResult,
};

use crate::AppServer;

impl AppServer {
    pub(crate) async fn handle_team_start(
        &self,
        params: TeamStartParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let team = self
            .runtime
            .start_team(TeamStartRequest {
                name: params.name,
                workspace: params.workspace,
                provider: params.provider,
                model: params.model,
            })
            .await
            .map_err(crate::server::internal_error)?;
        Ok(serde_json::to_value(TeamStartResult { team }).unwrap())
    }

    pub(crate) async fn handle_team_list(&self) -> Result<serde_json::Value, JsonRpcError> {
        let teams = self.runtime.list_teams().await;
        Ok(serde_json::to_value(TeamListResult { teams }).unwrap())
    }

    pub(crate) async fn handle_team_read(
        &self,
        params: TeamReadParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let team = self
            .runtime
            .read_team(&params.team_id)
            .await
            .map_err(crate::server::internal_error)?;
        Ok(serde_json::to_value(TeamReadResult { team }).unwrap())
    }

    pub(crate) async fn handle_team_channel_message(
        &self,
        params: TeamChannelMessageParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let outcome = self
            .runtime
            .append_team_channel_message(TeamChannelMessageRequest {
                team_id: params.team_id,
                channel_id: params.channel_id,
                text: params.text,
                author_member_id: params.author_member_id,
                project_context: params.project_context,
                thread_ts: params.thread_ts,
            })
            .await
            .map_err(crate::server::internal_error)?;
        Ok(serde_json::to_value(TeamChannelMessageResult {
            team: outcome.team,
            message: outcome.message,
        })
        .unwrap())
    }

    pub(crate) async fn handle_team_member_message(
        &self,
        params: TeamMemberMessageParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let outcome = self
            .runtime
            .append_team_member_message(TeamMemberMessageRequest {
                team_id: params.team_id,
                member_id: params.member_id,
                channel_id: params.channel_id,
                text: params.text,
            })
            .await
            .map_err(crate::server::internal_error)?;
        Ok(serde_json::to_value(TeamMemberMessageResult {
            team: outcome.team,
            member: outcome.member,
            message: outcome.message,
            turn_id: outcome.turn_id,
        })
        .unwrap())
    }

    pub(crate) async fn handle_team_member_interrupt(
        &self,
        params: TeamMemberInterruptParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let outcome = self
            .runtime
            .interrupt_team_member(&params.team_id, &params.member_id)
            .await
            .map_err(crate::server::internal_error)?;
        Ok(serde_json::to_value(TeamMemberInterruptResult {
            team: outcome.team,
            member: outcome.member,
        })
        .unwrap())
    }

    pub(crate) async fn handle_team_scheduler_set(
        &self,
        params: TeamSchedulerSetParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let team = self
            .runtime
            .set_team_scheduler(&params.team_id, params.running)
            .await
            .map_err(crate::server::internal_error)?;
        Ok(serde_json::to_value(TeamSchedulerSetResult { team }).unwrap())
    }

    pub(crate) async fn handle_team_cleanup(
        &self,
        params: TeamCleanupParams,
    ) -> Result<serde_json::Value, JsonRpcError> {
        let cleaned = self
            .runtime
            .cleanup_team(&params.team_id)
            .await
            .map_err(crate::server::internal_error)?;
        Ok(serde_json::to_value(TeamCleanupResult {
            team_id: params.team_id,
            cleaned,
        })
        .unwrap())
    }
}
