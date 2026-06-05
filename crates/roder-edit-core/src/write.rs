use crate::hunks::{EditHunk, text_edit_hunk};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteFileOutcome {
    pub path: String,
    pub content: String,
    pub hunks: Vec<EditHunk>,
}

pub fn write_file(
    path: impl Into<String>,
    previous: Option<&str>,
    content: String,
) -> WriteFileOutcome {
    let path = path.into();
    let hunks = previous
        .map(|old| vec![text_edit_hunk(&path, old, &content, 0)])
        .unwrap_or_default();
    WriteFileOutcome {
        path,
        content,
        hunks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_existing_file_produces_hunk() {
        let outcome = write_file("a.txt", Some("old"), "new".to_string());
        assert_eq!(outcome.hunks.len(), 1);
    }
}
