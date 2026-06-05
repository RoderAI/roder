pub mod fuzzy;
pub mod hunks;
pub mod patch;
pub mod post_edit;
pub mod read;
pub mod replace;
pub mod write;

use serde::{Deserialize, Serialize};

pub use hunks::{EditHunk, HunkDiffLine, HunkDiffLineKind};
pub use patch::{
    CodexPatchChange, CodexPatchOp, apply_codex_patch_to_workspace, parse_codex_patch,
};
pub use read::{ReadFormatOptions, format_line_numbered_read};
pub use replace::{EditApplyError, EditMatchMode, EditOptions, apply_edit, apply_multi_edit};
pub use write::{WriteFileOutcome, write_file};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReadRequest {
    pub path: String,
    pub start_line: Option<usize>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WriteRequest {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EditRequest {
    pub path: String,
    pub old_string: String,
    pub new_string: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MultiEditRequest {
    pub path: String,
    pub edits: Vec<TextEdit>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApplyPatchRequest {
    pub patch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TextEdit {
    pub old_string: String,
    pub new_string: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EditToolResult {
    pub path: String,
    pub replacements: usize,
    pub hunks: Vec<EditHunk>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EditToolError {
    pub kind: String,
    pub message: String,
    pub edit: Option<usize>,
    pub candidates: Vec<fuzzy::FuzzyCandidate>,
}
