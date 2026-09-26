use crate::hunks::{EditHunk, HunkDiffLine, HunkDiffLineKind};
use serde::{Deserialize, Serialize};
use similar::{ChangeTag, TextDiff};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchFileChange {
    pub path: String,
    pub kind: char,
    pub old_content: Option<String>,
    pub new_content: Option<String>,
    pub move_from: Option<String>,
    pub overwritten_move_content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchOutcome {
    pub summary: String,
    pub changes: Vec<PatchFileChange>,
    /// False when a failed write or unreadable preimage makes the delta uncertain.
    pub exact: bool,
}

impl Default for PatchOutcome {
    fn default() -> Self {
        Self {
            summary: String::new(),
            changes: Vec::new(),
            exact: true,
        }
    }
}

impl PatchOutcome {
    pub fn finish(mut self) -> Self {
        self.summary = super::patch_summary(
            &self
                .changes
                .iter()
                .map(|change| (change.kind, change.path.clone()))
                .collect::<Vec<_>>(),
        );
        self
    }

    pub fn hunks(&self) -> Vec<EditHunk> {
        self.changes
            .iter()
            .enumerate()
            .map(|(index, change)| {
                let old = change.old_content.as_deref().unwrap_or_default();
                let new = change.new_content.as_deref().unwrap_or_default();
                let text_diff = TextDiff::from_lines(old, new);
                let groups = text_diff.grouped_ops(1);
                let diff = groups
                    .iter()
                    .flatten()
                    .flat_map(|operation| text_diff.iter_changes(operation))
                    .map(|line| HunkDiffLine {
                        kind: match line.tag() {
                            ChangeTag::Equal => HunkDiffLineKind::Context,
                            ChangeTag::Delete => HunkDiffLineKind::Removed,
                            ChangeTag::Insert => HunkDiffLineKind::Added,
                        },
                        text: line
                            .value()
                            .trim_end_matches('\n')
                            .trim_end_matches('\r')
                            .to_string(),
                        old_line: line.old_index().map(|index| index as u32 + 1),
                        new_line: line.new_index().map(|index| index as u32 + 1),
                    })
                    .collect();
                EditHunk {
                    id: Some(format!("hunk-{}", index + 1)),
                    path: change.path.clone(),
                    old_start: 1,
                    new_start: 1,
                    old_lines: old.lines().count() as u32,
                    new_lines: new.lines().count() as u32,
                    diff,
                    reverse_patch: (self.exact && change.reverse_is_representable())
                        .then(|| change.reverse_patch()),
                }
            })
            .collect()
    }
}

impl PatchFileChange {
    fn reverse_is_representable(&self) -> bool {
        [&self.old_content, &self.overwritten_move_content]
            .into_iter()
            .flatten()
            .all(|text| !text.contains('\r') && (text.is_empty() || text.ends_with('\n')))
    }

    fn reverse_patch(&self) -> String {
        fn restore(patch: &mut String, path: &str, content: Option<&str>) {
            if let Some(content) = content {
                patch.push_str(&format!("*** Add File: {path}\n"));
                for line in content.lines() {
                    patch.push('+');
                    patch.push_str(line);
                    patch.push('\n');
                }
            } else {
                patch.push_str(&format!("*** Delete File: {path}\n"));
            }
        }
        let mut patch = String::from("*** Begin Patch\n");
        if let Some(source) = &self.move_from {
            restore(&mut patch, source, self.old_content.as_deref());
            restore(
                &mut patch,
                &self.path,
                self.overwritten_move_content.as_deref(),
            );
        } else {
            restore(&mut patch, &self.path, self.old_content.as_deref());
        }
        patch.push_str("*** End Patch\n");
        patch
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct PatchFailure {
    pub message: String,
    pub partial: PatchOutcome,
}
