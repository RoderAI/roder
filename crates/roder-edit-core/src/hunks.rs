use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EditHunk {
    pub id: Option<String>,
    pub path: String,
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub diff: Vec<HunkDiffLine>,
    pub reverse_patch: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HunkDiffLine {
    pub kind: HunkDiffLineKind,
    pub text: String,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum HunkDiffLineKind {
    Context,
    Added,
    Removed,
}

pub fn text_edit_hunk(
    path: impl Into<String>,
    old_text: &str,
    new_text: &str,
    index: usize,
) -> EditHunk {
    let path = path.into();
    let old_lines = old_text.lines().map(str::to_string).collect::<Vec<_>>();
    let new_lines = new_text.lines().map(str::to_string).collect::<Vec<_>>();
    lines_hunk(path, old_lines, new_lines, index)
}

pub fn lines_hunk(
    path: impl Into<String>,
    old_lines: Vec<String>,
    new_lines: Vec<String>,
    index: usize,
) -> EditHunk {
    let path = path.into();
    let mut diff = Vec::new();
    for (old_line, line) in (1u32..).zip(old_lines.iter()) {
        diff.push(HunkDiffLine {
            kind: HunkDiffLineKind::Removed,
            text: line.clone(),
            old_line: Some(old_line),
            new_line: None,
        });
    }
    for (new_line, line) in (1u32..).zip(new_lines.iter()) {
        diff.push(HunkDiffLine {
            kind: HunkDiffLineKind::Added,
            text: line.clone(),
            old_line: None,
            new_line: Some(new_line),
        });
    }
    EditHunk {
        id: Some(format!("hunk-{}", index + 1)),
        path: path.clone(),
        old_start: 1,
        old_lines: old_lines.len() as u32,
        new_start: 1,
        new_lines: new_lines.len() as u32,
        diff,
        reverse_patch: Some(reverse_codex_patch(&path, &old_lines, &new_lines)),
    }
}

pub fn reverse_codex_patch(path: &str, old_lines: &[String], new_lines: &[String]) -> String {
    let mut patch = String::from("*** Begin Patch\n");
    patch.push_str(&format!("*** Update File: {path}\n@@\n"));
    for line in new_lines {
        patch.push('-');
        patch.push_str(line);
        patch.push('\n');
    }
    for line in old_lines {
        patch.push('+');
        patch.push_str(line);
        patch.push('\n');
    }
    patch.push_str("*** End Patch\n");
    patch
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hunk_contains_reverse_patch() {
        let hunk = text_edit_hunk("src/lib.rs", "old", "new", 0);
        assert_eq!(hunk.old_lines, 1);
        assert_eq!(hunk.new_lines, 1);
        assert!(hunk.reverse_patch.unwrap().contains("-new\n+old"));
    }
}
