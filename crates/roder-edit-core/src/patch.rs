//! Codex apply_patch grammar, streaming parser, ordered matching, and execution.
mod filesystem;
mod outcome;
mod parser;
mod seek_sequence;
mod streaming;
mod text_file;
mod update;

pub use filesystem::{
    apply_codex_patch_to_workspace, apply_codex_patch_to_workspace_with_external_paths,
};
pub use outcome::{PatchFailure, PatchFileChange, PatchOutcome};
pub use parser::{Hunk, ParseError, ParsedPatch, UpdateFileChunk, parse_patch};
pub use streaming::StreamingPatchParser;
pub use update::derive_new_contents;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ApplyPatchFileUpdateMode {
    #[default]
    NormalizeToLf,
    PreserveLineEndings,
}

pub fn is_codex_patch(patch: &str) -> bool {
    let text = patch.trim_start();
    text.starts_with("*** Begin Patch")
        || text.starts_with("<<EOF")
        || text.starts_with("<<'EOF'")
        || text.starts_with("<<\"EOF\"")
}

pub fn codex_patch_hunks(patch: &str) -> anyhow::Result<Vec<crate::hunks::EditHunk>> {
    let parsed = parse_patch(patch)?;
    let mut records = Vec::new();
    for hunk in parsed.hunks {
        let path = hunk.path().to_string_lossy().into_owned();
        match hunk {
            Hunk::AddFile { contents, .. } => records.push(crate::hunks::lines_hunk(
                path,
                Vec::new(),
                contents.lines().map(String::from).collect(),
                records.len(),
            )),
            Hunk::DeleteFile { .. } => records.push(crate::hunks::lines_hunk(
                path,
                Vec::new(),
                Vec::new(),
                records.len(),
            )),
            Hunk::UpdateFile { chunks, .. } => {
                for chunk in chunks {
                    records.push(crate::hunks::lines_hunk(
                        path.clone(),
                        chunk.old_lines,
                        chunk.new_lines,
                        records.len(),
                    ));
                }
            }
        }
    }
    Ok(records)
}

/// Codex groups successful operations as added, modified, then deleted paths.
pub fn patch_summary(affected: &[(char, String)]) -> String {
    let mut output = String::from("Success. Updated the following files:\n");
    for kind in ['A', 'M', 'D'] {
        for (op, path) in affected {
            if *op == kind {
                output.push_str(&format!("{op} {path}\n"));
            }
        }
    }
    output
}

#[cfg(test)]
#[path = "patch/tests.rs"]
mod tests;
