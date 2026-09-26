use super::{ApplyPatchFileUpdateMode, Hunk, derive_new_contents, parse_patch};
use anyhow::{Context, bail};
use std::path::{Component, Path, PathBuf};

pub fn apply_codex_patch_to_workspace(
    root: &Path,
    patch: &str,
) -> anyhow::Result<super::PatchOutcome> {
    apply_codex_patch_to_workspace_with_external_paths(root, patch, false)
}

pub fn apply_codex_patch_to_workspace_with_external_paths(
    root: &Path,
    patch: &str,
    allow_external_paths: bool,
) -> anyhow::Result<super::PatchOutcome> {
    let root = root
        .canonicalize()
        .with_context(|| format!("workspace root does not exist: {}", root.display()))?;
    let parsed = parse_patch(patch)?;
    if parsed.environment_id.is_some() {
        bail!("apply_patch environment_id is unavailable in this session");
    }
    if parsed.hunks.is_empty() {
        bail!("No files were modified.");
    }
    let mut prepared = Vec::new();
    let mut sources = std::collections::HashSet::new();
    for hunk in parsed.hunks {
        let source = match &hunk {
            Hunk::AddFile { path, .. }
            | Hunk::DeleteFile { path }
            | Hunk::UpdateFile { path, .. } => path,
        };
        let path = resolve_for_write(&root, &source.to_string_lossy(), allow_external_paths)?;
        if !sources.insert(path.clone()) {
            return Err(super::ParseError::InvalidPatchError(format!(
                "multiple operations target {}",
                path.display()
            ))
            .into());
        }
        let display = source.to_string_lossy().into_owned();
        let target = match &hunk {
            Hunk::AddFile { .. } => None,
            Hunk::DeleteFile { .. } => {
                std::fs::read_to_string(&path)
                    .with_context(|| format!("Failed to read {}", path.display()))?;
                None
            }
            Hunk::UpdateFile {
                move_path, chunks, ..
            } => {
                let before = read_update(&path)?;
                derive_new_contents(
                    &before,
                    &path.to_string_lossy(),
                    chunks,
                    ApplyPatchFileUpdateMode::default(),
                )?;
                move_path
                    .as_ref()
                    .map(|dest| {
                        resolve_for_write(&root, &dest.to_string_lossy(), allow_external_paths)
                            .map(|target| (target, dest.to_string_lossy().into_owned()))
                    })
                    .transpose()?
            }
        };
        prepared.push((hunk, path, display, target));
    }
    let mut outcome = super::PatchOutcome::default();
    for (hunk, path, display, target) in prepared {
        outcome.exact &= !std::fs::symlink_metadata(&path)
            .is_ok_and(|metadata| metadata.file_type().is_symlink());
        let result: anyhow::Result<()> = (|| {
            match hunk {
                Hunk::AddFile { contents, .. } => {
                    let (old_content, exact) = optional_text(&path);
                    outcome.exact &= exact;
                    if let Err(error) = write_file(&path, &contents) {
                        outcome.exact = false;
                        return Err(error);
                    }
                    outcome.changes.push(super::PatchFileChange {
                        path: display,
                        kind: 'A',
                        old_content,
                        new_content: Some(contents),
                        move_from: None,
                        overwritten_move_content: None,
                    });
                }
                Hunk::DeleteFile { .. } => {
                    let old_content = std::fs::read_to_string(&path)?;
                    std::fs::remove_file(&path)
                        .with_context(|| format!("Failed to delete file {}", path.display()))?;
                    outcome.changes.push(super::PatchFileChange {
                        path: display,
                        kind: 'D',
                        old_content: Some(old_content),
                        new_content: None,
                        move_from: None,
                        overwritten_move_content: None,
                    });
                }
                Hunk::UpdateFile { chunks, .. } => {
                    // Re-read at execution just as Codex's patch process does, so
                    // earlier moves and concurrent filesystem changes are respected.
                    let old_content = read_update(&path)?;
                    let contents = derive_new_contents(
                        &old_content,
                        &path.to_string_lossy(),
                        &chunks,
                        ApplyPatchFileUpdateMode::default(),
                    )?;
                    if let Some((target, target_display)) = target {
                        outcome.exact &= !std::fs::symlink_metadata(&target)
                            .is_ok_and(|metadata| metadata.file_type().is_symlink());
                        let (overwritten, exact) = optional_text(&target);
                        outcome.exact &= exact;
                        if let Err(error) = write_file(&target, &contents) {
                            outcome.exact = false;
                            return Err(error);
                        }
                        let change_index = outcome.changes.len();
                        outcome.changes.push(super::PatchFileChange {
                            path: target_display.clone(),
                            kind: 'A',
                            old_content: overwritten.clone(),
                            new_content: Some(contents.clone()),
                            move_from: None,
                            overwritten_move_content: None,
                        });
                        std::fs::remove_file(&path).with_context(|| {
                            format!("Failed to remove original {}", path.display())
                        })?;
                        outcome.changes[change_index] = super::PatchFileChange {
                            path: target_display,
                            kind: 'M',
                            old_content: Some(old_content),
                            new_content: Some(contents),
                            move_from: Some(display),
                            overwritten_move_content: overwritten,
                        };
                    } else {
                        if let Err(error) = write_file(&path, &contents) {
                            outcome.exact = false;
                            return Err(error);
                        }
                        outcome.changes.push(super::PatchFileChange {
                            path: display,
                            kind: 'M',
                            old_content: Some(old_content),
                            new_content: Some(contents),
                            move_from: None,
                            overwritten_move_content: None,
                        });
                    }
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            return Err(super::PatchFailure {
                message: format!("{error:#}"),
                partial: outcome.finish(),
            }
            .into());
        }
    }
    Ok(outcome.finish())
}

fn read_update(path: &Path) -> anyhow::Result<String> {
    std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read file to update {}", path.display()))
}

fn optional_text(path: &Path) -> (Option<String>, bool) {
    match std::fs::read_to_string(path) {
        Ok(text) => (Some(text), true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (None, true),
        Err(_) => (None, false),
    }
}

fn write_file(path: &Path, contents: &str) -> anyhow::Result<()> {
    // Codex retries a missing-parent write after creating directories.
    match std::fs::write(path, contents) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, contents)
                .with_context(|| format!("Failed to write file {}", path.display()))
        }
        Err(err) => Err(err).with_context(|| format!("Failed to write file {}", path.display())),
    }
}

fn resolve_for_write(
    root: &Path,
    input: &str,
    allow_external_paths: bool,
) -> anyhow::Result<PathBuf> {
    let candidate = if Path::new(input).is_absolute() {
        PathBuf::from(input)
    } else {
        root.join(input)
    };
    let root = root
        .canonicalize()
        .with_context(|| format!("workspace root does not exist: {}", root.display()))?;
    let normalized = normalize(candidate)?;
    if allow_external_paths {
        return Ok(normalized);
    }
    let mut ancestor = normalized.as_path();
    let mut missing = Vec::new();
    while match std::fs::symlink_metadata(ancestor) {
        Ok(_) => false,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(error) => return Err(error.into()),
    } {
        missing.push(
            ancestor
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("path is required"))?
                .to_os_string(),
        );
        ancestor = ancestor
            .parent()
            .ok_or_else(|| anyhow::anyhow!("path is required"))?;
    }
    let mut normalized_for_check = ancestor.canonicalize()?;
    for component in missing.into_iter().rev() {
        normalized_for_check.push(component);
    }
    if !allow_external_paths && !normalized_for_check.starts_with(&root) {
        bail!(
            "path {} is outside workspace {}",
            normalized_for_check.display(),
            root.display()
        );
    }
    // Canonicalization checks authorization; preserve the path for operations
    // such as unlinking a symlink, matching Codex's filesystem semantics.
    Ok(normalized)
}

fn normalize(path: PathBuf) -> anyhow::Result<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::Normal(part) => normalized.push(part),
            Component::ParentDir => {
                if !normalized.pop() {
                    bail!("path escapes filesystem root");
                }
            }
        }
    }
    Ok(normalized)
}
