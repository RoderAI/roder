use serde::{Deserialize, Serialize};

use crate::fuzzy::{
    FuzzyCandidate, diagnostic_candidates, normalized_unique_match_range,
    strip_line_number_prefixes,
};
use crate::hunks::{EditHunk, text_edit_hunk};
use crate::{EditToolResult, TextEdit};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EditMatchMode {
    Off,
    Diagnose,
    ApplySafe,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct EditOptions {
    pub fuzzy: EditMatchMode,
    pub strip_line_numbers: bool,
    /**
     * Bounded indentation normalization for inserted/replaced code: when the
     * replaced text is uniformly indented and the replacement omitted that
     * indentation entirely, shift the replacement right to match. Off by
     * default; hosts opt in per call.
     */
    #[serde(default)]
    pub reindent_inserted: bool,
}

impl Default for EditOptions {
    fn default() -> Self {
        Self {
            fuzzy: EditMatchMode::Diagnose,
            strip_line_numbers: true,
            reindent_inserted: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum EditApplyError {
    OldStringNotFound {
        edit: Option<usize>,
        candidates: Vec<FuzzyCandidate>,
    },
    OldStringAmbiguous {
        edit: Option<usize>,
        occurrences: usize,
        candidates: Vec<FuzzyCandidate>,
    },
}

pub fn apply_edit(
    path: impl Into<String>,
    text: &str,
    old_string: &str,
    new_string: &str,
    options: EditOptions,
) -> Result<(String, EditToolResult), EditApplyError> {
    let edit = TextEdit {
        old_string: old_string.to_string(),
        new_string: new_string.to_string(),
    };
    apply_multi_edit(path, text, &[edit], options)
}

pub fn apply_multi_edit(
    path: impl Into<String>,
    text: &str,
    edits: &[TextEdit],
    options: EditOptions,
) -> Result<(String, EditToolResult), EditApplyError> {
    let path = path.into();
    let mut updated = text.to_string();
    let mut hunks = Vec::new();
    for (index, edit) in edits.iter().enumerate() {
        let old_string = if options.strip_line_numbers {
            strip_line_number_prefixes(&edit.old_string)
        } else {
            edit.old_string.clone()
        };
        let matches = match_positions(&updated, &old_string);
        let range = match matches.as_slice() {
            [position] => *position..*position + old_string.len(),
            [] => match options.fuzzy {
                EditMatchMode::Off | EditMatchMode::Diagnose => {
                    return Err(EditApplyError::OldStringNotFound {
                        edit: Some(index),
                        candidates: diagnostic_candidates(&updated, &old_string, 3),
                    });
                }
                EditMatchMode::ApplySafe => normalized_unique_match_range(&updated, &old_string)
                    .ok_or_else(|| EditApplyError::OldStringNotFound {
                        edit: Some(index),
                        candidates: diagnostic_candidates(&updated, &old_string, 3),
                    })?,
            },
            _ => {
                return Err(EditApplyError::OldStringAmbiguous {
                    edit: Some(index),
                    occurrences: matches.len(),
                    candidates: diagnostic_candidates(&updated, &old_string, 3),
                });
            }
        };
        // Use the actual matched text (which may differ from old_string under
        // fuzzy recovery) for reindent context and hunk/reverse-patch data.
        let matched_old = updated[range.clone()].to_string();
        let new_string = if options.reindent_inserted {
            crate::post_edit::normalize_inserted_indentation(&matched_old, &edit.new_string)
        } else {
            edit.new_string.clone()
        };
        updated.replace_range(range, &new_string);
        hunks.push(text_edit_hunk(&path, &matched_old, &new_string, index));
    }
    Ok((
        updated,
        EditToolResult {
            path,
            replacements: edits.len(),
            hunks,
        },
    ))
}

fn match_positions(haystack: &str, needle: &str) -> Vec<usize> {
    if needle.is_empty() {
        return Vec::new();
    }
    haystack
        .match_indices(needle)
        .map(|(index, _)| index)
        .collect()
}

pub fn hunks_for_edits(path: impl Into<String>, edits: &[TextEdit]) -> Vec<EditHunk> {
    let path = path.into();
    edits
        .iter()
        .enumerate()
        .map(|(index, edit)| text_edit_hunk(&path, &edit.old_string, &edit.new_string, index))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_exact_edit_once() {
        let (updated, outcome) = apply_edit(
            "src/lib.rs",
            "fn main() { true }",
            "true",
            "false",
            EditOptions::default(),
        )
        .unwrap();
        assert_eq!(updated, "fn main() { false }");
        assert_eq!(outcome.replacements, 1);
        assert_eq!(outcome.hunks.len(), 1);
    }

    #[test]
    fn refuses_ambiguous_edit() {
        let err = apply_edit("x", "foo foo", "foo", "bar", EditOptions::default()).unwrap_err();
        assert!(matches!(
            err,
            EditApplyError::OldStringAmbiguous { occurrences: 2, .. }
        ));
    }

    #[test]
    fn strips_line_numbers_before_matching() {
        let (updated, _) = apply_edit(
            "x",
            "foo\nbar",
            "1: foo\n2: bar",
            "baz",
            EditOptions::default(),
        )
        .unwrap();
        assert_eq!(updated, "baz");
    }
}
