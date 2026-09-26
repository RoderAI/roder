//! Proposed patch changes emitted during generation; these are not evidence
//! of a filesystem write or approval to execute a tool.
use crate::events::{ThreadId, TurnId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchProgress {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub tool_id: String,
    pub patch: String,
    pub changes: Vec<ProposedPatchChange>,
    pub complete: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposedPatchChange {
    pub path: String,
    pub change_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub move_to: Option<String>,
    /// Requested context/removal lines, not the complete source file.
    pub old_lines: Vec<String>,
    /// Requested context/addition lines, not the complete resulting file.
    pub new_lines: Vec<String>,
}
