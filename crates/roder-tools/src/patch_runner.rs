use crate::workspace::Workspace;
use roder_api::remote_runner::{
    RemoteRunnerSession, RunnerCommandRequest, RunnerFileReadRequest, RunnerFileWriteRequest,
};
use roder_edit_core::patch::{
    ApplyPatchFileUpdateMode, Hunk, ParseError, PatchFailure, PatchFileChange, PatchOutcome,
    derive_new_contents, parse_patch,
};

/// Execute the same parser and matching logic on the runner's filesystem.
pub(super) async fn apply(
    workspace: &Workspace,
    session: &dyn RemoteRunnerSession,
    patch: &str,
) -> anyhow::Result<PatchOutcome> {
    let parsed = parse_patch(patch)?;
    if parsed.environment_id.is_some() {
        anyhow::bail!("apply_patch environment_id is unavailable in this session");
    }
    if parsed.hunks.is_empty() {
        anyhow::bail!("No files were modified.");
    }
    let mut prepared = Vec::new();
    let mut sources = std::collections::HashSet::new();
    for hunk in parsed.hunks {
        let source = match &hunk {
            Hunk::AddFile { path, .. }
            | Hunk::DeleteFile { path }
            | Hunk::UpdateFile { path, .. } => path,
        };
        let rel = workspace.display(&workspace.resolve_for_write(&source.to_string_lossy())?);
        if !sources.insert(rel.clone()) {
            return Err(
                ParseError::InvalidPatchError(format!("multiple operations target {rel}")).into(),
            );
        }
        let display = source.to_string_lossy().into_owned();
        let target = match &hunk {
            Hunk::AddFile { .. } => None,
            Hunk::DeleteFile { .. } => {
                read_text(session, &rel).await?;
                None
            }
            Hunk::UpdateFile {
                move_path, chunks, ..
            } => {
                derive_new_contents(
                    &read_text(session, &rel).await?,
                    &display,
                    chunks,
                    ApplyPatchFileUpdateMode::default(),
                )?;
                move_path
                    .as_ref()
                    .map(|dest| {
                        workspace
                            .resolve_for_write(&dest.to_string_lossy())
                            .map(|resolved| {
                                (
                                    workspace.display(&resolved),
                                    dest.to_string_lossy().into_owned(),
                                )
                            })
                    })
                    .transpose()?
            }
        };
        prepared.push((hunk, rel, display, target));
    }
    let mut outcome = PatchOutcome::default();
    for (hunk, rel, display, target) in prepared {
        let result: anyhow::Result<()> = async {
            match hunk {
                Hunk::AddFile { contents, .. } => {
                    let (old_content, exact) = optional_text(workspace, session, &rel).await;
                    outcome.exact &= exact;
                    if let Err(error) = write(session, &rel, &contents).await {
                        outcome.exact = false;
                        return Err(error);
                    }
                    outcome.changes.push(PatchFileChange {
                        path: display,
                        kind: 'A',
                        old_content,
                        new_content: Some(contents),
                        move_from: None,
                        overwritten_move_content: None,
                    });
                }
                Hunk::DeleteFile { .. } => {
                    let old_content = read_text(session, &rel).await?;
                    if let Err(error) = remove(workspace, session, &rel).await {
                        outcome.exact = false;
                        return Err(error);
                    }
                    outcome.changes.push(PatchFileChange {
                        path: display,
                        kind: 'D',
                        old_content: Some(old_content),
                        new_content: None,
                        move_from: None,
                        overwritten_move_content: None,
                    });
                }
                Hunk::UpdateFile { chunks, .. } => {
                    let old_content = read_text(session, &rel).await?;
                    let contents = derive_new_contents(
                        &old_content,
                        &display,
                        &chunks,
                        ApplyPatchFileUpdateMode::default(),
                    )?;
                    if let Some((target, target_display)) = target {
                        let (overwritten, exact) = optional_text(workspace, session, &target).await;
                        outcome.exact &= exact;
                        if let Err(error) = write(session, &target, &contents).await {
                            outcome.exact = false;
                            return Err(error);
                        }
                        let index = outcome.changes.len();
                        outcome.changes.push(PatchFileChange {
                            path: target_display.clone(),
                            kind: 'A',
                            old_content: overwritten.clone(),
                            new_content: Some(contents.clone()),
                            move_from: None,
                            overwritten_move_content: None,
                        });
                        if let Err(error) = remove(workspace, session, &rel).await {
                            outcome.exact = false;
                            return Err(error);
                        }
                        outcome.changes[index] = PatchFileChange {
                            path: target_display,
                            kind: 'M',
                            old_content: Some(old_content),
                            new_content: Some(contents),
                            move_from: Some(display),
                            overwritten_move_content: overwritten,
                        };
                    } else {
                        if let Err(error) = write(session, &rel, &contents).await {
                            outcome.exact = false;
                            return Err(error);
                        }
                        outcome.changes.push(PatchFileChange {
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
        }
        .await;
        if let Err(error) = result {
            return Err(PatchFailure {
                message: format!("{error:#}"),
                partial: outcome.finish(),
            }
            .into());
        }
    }
    Ok(outcome.finish())
}

async fn read_text(session: &dyn RemoteRunnerSession, path: &str) -> anyhow::Result<String> {
    String::from_utf8(
        session
            .read_file(RunnerFileReadRequest { path: path.into() })
            .await?
            .contents,
    )
    .map_err(Into::into)
}

async fn write(
    session: &dyn RemoteRunnerSession,
    path: &str,
    contents: &str,
) -> anyhow::Result<()> {
    session
        .write_file(RunnerFileWriteRequest {
            path: path.into(),
            contents: contents.as_bytes().to_vec(),
        })
        .await
}

async fn optional_text(
    workspace: &Workspace,
    session: &dyn RemoteRunnerSession,
    path: &str,
) -> (Option<String>, bool) {
    if let Ok(text) = read_text(session, path).await {
        return (Some(text), true);
    }
    let exists = session
        .run_command(RunnerCommandRequest {
            command_id: "apply-patch-stat".into(),
            program: "test".into(),
            args: vec!["-e".into(), path.into()],
            cwd: Some(workspace.root().to_path_buf()),
            env: Vec::new(),
            timeout_ms: None,
        })
        .await;
    (None, exists.is_ok_and(|result| result.exit_code == Some(1)))
}

async fn remove(
    workspace: &Workspace,
    session: &dyn RemoteRunnerSession,
    path: &str,
) -> anyhow::Result<()> {
    let output = session
        .run_command(RunnerCommandRequest {
            command_id: "apply-patch-delete".to_string(),
            program: "rm".to_string(),
            args: vec!["--".to_string(), path.to_string()],
            cwd: Some(workspace.root().to_path_buf()),
            env: Vec::new(),
            timeout_ms: None,
        })
        .await?;
    if output.exit_code != Some(0) {
        anyhow::bail!("Failed to delete file {path}: {}", output.stderr.trim_end());
    }
    Ok(())
}

#[cfg(test)]
#[path = "patch_runner_tests.rs"]
mod tests;
