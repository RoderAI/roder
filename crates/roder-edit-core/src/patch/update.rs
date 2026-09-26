// Derived from OpenAI Codex apply-patch at 8b78f4796605bda8e31329537f5fef036e405e66.
// Copyright 2025 OpenAI. Licensed under Apache-2.0; see third-party/codex.
// Adapted for Roder: filesystem-independent parsing and matching.

use super::text_file::{Replacement, SourceFile};
use super::{ApplyPatchFileUpdateMode, UpdateFileChunk, seek_sequence};

/// Apply ordered, line-based chunks with Codex matching and newline semantics.
pub fn derive_new_contents(
    original_contents: &str,
    path: &str,
    chunks: &[UpdateFileChunk],
    mode: ApplyPatchFileUpdateMode,
) -> anyhow::Result<String> {
    match mode {
        ApplyPatchFileUpdateMode::NormalizeToLf => {
            let mut lines = original_contents
                .split('\n')
                .map(String::from)
                .collect::<Vec<_>>();
            if lines.last().is_some_and(String::is_empty) {
                lines.pop();
            }
            let replacements = compute_replacements(&lines, path, chunks, mode)?;
            let mut new_lines = apply_replacements(lines, &replacements);
            if !new_lines.last().is_some_and(String::is_empty) {
                new_lines.push(String::new());
            }
            Ok(new_lines.join("\n"))
        }
        ApplyPatchFileUpdateMode::PreserveLineEndings => {
            let mut source = SourceFile::parse(original_contents);
            let replacements = compute_replacements(&source.line_texts(), path, chunks, mode)?;
            source.apply_replacements(&replacements);
            Ok(source.into_contents())
        }
    }
}

/// Compute a list of replacements needed to transform `original_lines` into the
/// new lines, given the patch `chunks`. Each replacement is returned as
/// `(start_index, old_len, new_lines)`.
fn compute_replacements(
    original_lines: &[String],
    path: &str,
    chunks: &[UpdateFileChunk],
    update_file_mode: ApplyPatchFileUpdateMode,
) -> std::result::Result<Vec<Replacement>, anyhow::Error> {
    let mut replacements: Vec<Replacement> = Vec::new();
    let mut line_index: usize = 0;

    for chunk in chunks {
        // If a chunk has a `change_context`, we use seek_sequence to find it, then
        // adjust our `line_index` to continue from there.
        if let Some(ctx_line) = &chunk.change_context {
            if let Some(idx) = seek_sequence::seek_sequence(
                original_lines,
                std::slice::from_ref(ctx_line),
                line_index,
                /*eof*/ false,
                update_file_mode,
            ) {
                line_index = idx + 1;
            } else {
                return Err(anyhow::Error::msg(format!(
                    "Failed to find context '{ctx_line}' in {path}"
                )));
            }
        }

        if chunk.old_lines.is_empty() {
            // Preserve the legacy split representation's handling of a final
            // empty line. `SourceFile` only exposes real source lines, so its
            // insertion point is always after the final line.
            let insertion_idx = match update_file_mode {
                ApplyPatchFileUpdateMode::NormalizeToLf => {
                    if original_lines.last().is_some_and(String::is_empty) {
                        original_lines.len() - 1
                    } else {
                        original_lines.len()
                    }
                }
                ApplyPatchFileUpdateMode::PreserveLineEndings => original_lines.len(),
            };
            replacements.push((insertion_idx, 0, chunk.new_lines.clone()));
            continue;
        }

        // Otherwise, try to match the existing lines in the file with the old lines
        // from the chunk. If found, schedule that region for replacement.
        // Attempt to locate the `old_lines` verbatim within the file.  In many
        // real‑world diffs the last element of `old_lines` is an *empty* string
        // representing the terminating newline of the region being replaced.
        // This sentinel is not present in `original_lines` because `SourceFile`
        // stores the terminator on the preceding line rather than as an extra
        // trailing element. If a direct search fails and the pattern ends with
        // an empty string, retry without that final element so modifications
        // touching the end‑of‑file can be located reliably.

        let mut pattern: &[String] = &chunk.old_lines;
        let mut found = seek_sequence::seek_sequence(
            original_lines,
            pattern,
            line_index,
            chunk.is_end_of_file,
            update_file_mode,
        );

        let mut new_slice: &[String] = &chunk.new_lines;

        if found.is_none() && pattern.last().is_some_and(String::is_empty) {
            // Retry without the trailing empty line which represents the final
            // newline in the file.
            pattern = &pattern[..pattern.len() - 1];
            if new_slice.last().is_some_and(String::is_empty) {
                new_slice = &new_slice[..new_slice.len() - 1];
            }

            found = seek_sequence::seek_sequence(
                original_lines,
                pattern,
                line_index,
                chunk.is_end_of_file,
                update_file_mode,
            );
        }

        if let Some(start_idx) = found {
            match update_file_mode {
                ApplyPatchFileUpdateMode::NormalizeToLf => {
                    replacements.push((start_idx, pattern.len(), new_slice.to_vec()));
                }
                ApplyPatchFileUpdateMode::PreserveLineEndings => {
                    // Context lines occur in both sides of a patch chunk. Keep those
                    // original lines in place so their exact contents and terminators
                    // survive, especially when the file has mixed line endings.
                    let mut old_start = 0;
                    let mut new_start = 0;
                    for &(old_context, new_context) in &chunk.context_line_indices {
                        // A trailing empty context line can be removed from `pattern`
                        // and `new_slice` above when it represents the final newline.
                        if old_context >= pattern.len() || new_context >= new_slice.len() {
                            break;
                        }
                        if old_start != old_context || new_start != new_context {
                            replacements.push((
                                start_idx + old_start,
                                old_context - old_start,
                                new_slice[new_start..new_context].to_vec(),
                            ));
                        }
                        old_start = old_context + 1;
                        new_start = new_context + 1;
                    }
                    if old_start != pattern.len() || new_start != new_slice.len() {
                        replacements.push((
                            start_idx + old_start,
                            pattern.len() - old_start,
                            new_slice[new_start..].to_vec(),
                        ));
                    }
                }
            }
            line_index = start_idx + pattern.len();
        } else {
            return Err(anyhow::Error::msg(format!(
                "Failed to find expected lines in {}:\n{}",
                path,
                chunk.old_lines.join("\n"),
            )));
        }
    }

    replacements.sort_by_key(|(index, _, _)| *index);

    Ok(replacements)
}

/// Apply the `(start_index, old_len, new_lines)` replacements to `original_lines`,
/// returning the modified file contents as a vector of lines.
fn apply_replacements(mut lines: Vec<String>, replacements: &[Replacement]) -> Vec<String> {
    // We must apply replacements in descending order so that earlier replacements
    // don't shift the positions of later ones.
    for (start_idx, old_len, new_segment) in replacements.iter().rev() {
        let start_idx = *start_idx;
        let old_len = *old_len;

        // Remove old lines.
        for _ in 0..old_len {
            if start_idx < lines.len() {
                lines.remove(start_idx);
            }
        }

        // Insert new lines.
        for (offset, new_line) in new_segment.iter().enumerate() {
            lines.insert(start_idx + offset, new_line.clone());
        }
    }

    lines
}
