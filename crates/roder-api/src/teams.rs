use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::events::{ThreadId, TurnId};

pub type TeamId = String;
pub type TeamMemberId = String;
pub type TeamChannelId = String;
pub type TeamMessageId = String;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TeamSnapshot {
    pub id: TeamId,
    pub name: String,
    pub thread_id: ThreadId,
    pub workspace: Option<String>,
    pub provider: String,
    pub model: String,
    pub members: Vec<TeamMember>,
    pub channels: Vec<TeamChannel>,
    pub messages: Vec<TeamMessage>,
    pub aggressive_always_on: bool,
    pub scheduler_running: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TeamMember {
    pub id: TeamMemberId,
    pub role: String,
    pub display_name: String,
    pub thread_id: ThreadId,
    pub provider: String,
    pub model: String,
    pub worktree_path: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TeamChannel {
    pub id: TeamChannelId,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TeamMessage {
    pub id: TeamMessageId,
    pub text: String,
    pub author_kind: String,
    pub author_member_id: Option<TeamMemberId>,
    pub target_member_id: Option<TeamMemberId>,
    pub channel_id: Option<TeamChannelId>,
    pub project_context: Option<String>,
    pub thread_ts: Option<String>,
    pub turn_id: Option<TurnId>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}
