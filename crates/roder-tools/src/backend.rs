use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use regex::RegexBuilder;
use roder_api::remote_runner::{
    RemoteRunnerSession, RunnerCommandRequest, RunnerFileReadRequest, RunnerFileWriteRequest,
};
use roder_api::tools::ToolExecutionContext;
use roder_search::{INDEX_VERSION, SearchEngine, SearchMetadata, SearchOptions};

use crate::workspace::Workspace;

pub(crate) type WorkspaceBackendHandle = Arc<dyn WorkspaceBackend>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TextEdit {
    pub(crate) old_string: String,
    pub(crate) new_string: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EditOutcome {
    pub(crate) path: String,
    pub(crate) replacements: usize,
}

#[async_trait]
pub(crate) trait WorkspaceBackend: Send + Sync + 'static {
    async fn read_text(&self, path: &str) -> anyhow::Result<(String, String)>;

    async fn list_files(&self, path: &str) -> anyhow::Result<(String, Vec<String>)>;

    async fn write_text(&self, path: &str, content: String) -> anyhow::Result<String>;

    async fn edit_text(
        &self,
        path: &str,
        old_string: &str,
        new_string: &str,
    ) -> anyhow::Result<Option<EditOutcome>>;

    async fn multi_edit_text(
        &self,
        path: &str,
        edits: Vec<TextEdit>,
    ) -> anyhow::Result<Result<EditOutcome, usize>>;

    async fn grep_search(
        &self,
        options: SearchOptions,
    ) -> anyhow::Result<(String, Vec<String>, SearchMetadata)>;

    async fn glob(&self, pattern: &str) -> anyhow::Result<Vec<String>>;

    async fn apply_patch(&self, patch: &str) -> anyhow::Result<String>;
}

#[derive(Debug)]
pub(crate) struct LocalWorkspaceBackend {
    workspace: Workspace,
    searcher: Arc<Mutex<roder_search::WorkspaceSearcher>>,
}

/**
 * Workspace backend that executes every operation through a
 * `RemoteRunnerSession`. The `guard` only scopes and displays paths (use a
 * remote `Workspace` so resolution never touches local disk); content and
 * existence come from the runner.
 */
pub(crate) struct RunnerWorkspaceBackend {
    guard: Workspace,
    session: Arc<dyn RemoteRunnerSession>,
}

impl RunnerWorkspaceBackend {
    pub(crate) fn new(guard: Workspace, session: Arc<dyn RemoteRunnerSession>) -> Self {
        Self { guard, session }
    }
}

impl LocalWorkspaceBackend {
    pub(crate) fn new(workspace: Workspace) -> Self {
        let searcher = Arc::new(Mutex::new(roder_search::WorkspaceSearcher::new(
            workspace.root(),
        )));
        Self {
            workspace,
            searcher,
        }
    }

    fn invalidate_search_index(&self) -> anyhow::Result<()> {
        self.searcher
            .lock()
            .map_err(|_| anyhow::anyhow!("search index lock is poisoned"))?
            .invalidate();
        Ok(())
    }
}

pub(crate) fn backend_from_context_or_fallback(
    ctx: &ToolExecutionContext,
    fallback_workspace: &Workspace,
    fallback_backend: &WorkspaceBackendHandle,
) -> anyhow::Result<WorkspaceBackendHandle> {
    if let Some(remote) = ctx.handles.remote_workspace.as_ref() {
        let guard = Workspace::remote(remote.root.clone(), fallback_workspace.path_scope())?;
        return Ok(Arc::new(RunnerWorkspaceBackend::new(
            guard,
            remote.session.clone(),
        )));
    }
    let Some(handle) = ctx.handles.workspace.as_ref() else {
        return Ok(fallback_backend.clone());
    };
    let Some(root) = handle.workspace_root() else {
        return Ok(fallback_backend.clone());
    };
    let workspace = Workspace::new_with_scope(root, fallback_workspace.path_scope())?;
    if workspace.root() == fallback_workspace.root() {
        return Ok(fallback_backend.clone());
    }
    Ok(Arc::new(LocalWorkspaceBackend::new(workspace)))
}

#[async_trait]
impl WorkspaceBackend for LocalWorkspaceBackend {
    async fn read_text(&self, path: &str) -> anyhow::Result<(String, String)> {
        let path = self.workspace.resolve_existing(path)?;
        let text = std::fs::read_to_string(&path)?;
        Ok((self.workspace.display(&path), text))
    }

    async fn list_files(&self, path: &str) -> anyhow::Result<(String, Vec<String>)> {
        let path = self.workspace.resolve_existing(path)?;
        let mut names = Vec::new();
        for entry in std::fs::read_dir(&path)? {
            let entry = entry?;
            let mut name = entry.file_name().to_string_lossy().to_string();
            if entry.file_type()?.is_dir() {
                name.push('/');
            }
            names.push(name);
        }
        names.sort();
        Ok((self.workspace.display(&path), names))
    }

    async fn write_text(&self, path: &str, content: String) -> anyhow::Result<String> {
        let path = self.workspace.resolve_for_write(path)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, content)?;
        self.invalidate_search_index()?;
        Ok(self.workspace.display(&path))
    }

    async fn edit_text(
        &self,
        path: &str,
        old_string: &str,
        new_string: &str,
    ) -> anyhow::Result<Option<EditOutcome>> {
        let path = self.workspace.resolve_existing(path)?;
        let text = std::fs::read_to_string(&path)?;
        let rel = self.workspace.display(&path);
        let (updated, outcome) = match roder_edit_core::apply_edit(
            rel.clone(),
            &text,
            old_string,
            new_string,
            roder_edit_core::EditOptions {
                fuzzy: roder_edit_core::EditMatchMode::Off,
                strip_line_numbers: false,
                reindent_inserted: false,
            },
        ) {
            Ok(result) => result,
            Err(_) => return Ok(None),
        };
        std::fs::write(&path, updated)?;
        self.invalidate_search_index()?;
        Ok(Some(EditOutcome {
            path: outcome.path,
            replacements: outcome.replacements,
        }))
    }

    async fn multi_edit_text(
        &self,
        path: &str,
        edits: Vec<TextEdit>,
    ) -> anyhow::Result<Result<EditOutcome, usize>> {
        let path = self.workspace.resolve_existing(path)?;
        let text = std::fs::read_to_string(&path)?;
        let rel = self.workspace.display(&path);
        let core_edits = edits
            .iter()
            .map(|edit| roder_edit_core::TextEdit {
                old_string: edit.old_string.clone(),
                new_string: edit.new_string.clone(),
            })
            .collect::<Vec<_>>();
        let (updated, outcome) = match roder_edit_core::apply_multi_edit(
            rel.clone(),
            &text,
            &core_edits,
            roder_edit_core::EditOptions {
                fuzzy: roder_edit_core::EditMatchMode::Off,
                strip_line_numbers: false,
                reindent_inserted: false,
            },
        ) {
            Ok(result) => result,
            Err(roder_edit_core::EditApplyError::OldStringNotFound { edit, .. }) => {
                return Ok(Err(edit.unwrap_or(0)));
            }
            Err(roder_edit_core::EditApplyError::OldStringAmbiguous { edit, .. }) => {
                return Ok(Err(edit.unwrap_or(0)));
            }
        };
        std::fs::write(&path, updated)?;
        self.invalidate_search_index()?;
        Ok(Ok(EditOutcome {
            path: outcome.path,
            replacements: outcome.replacements,
        }))
    }

    async fn grep_search(
        &self,
        options: SearchOptions,
    ) -> anyhow::Result<(String, Vec<String>, SearchMetadata)> {
        let workspace = self.workspace.clone();
        let searcher = self.searcher.clone();
        tokio::task::spawn_blocking(move || {
            let input_path = options.path.to_string_lossy().to_string();
            let start = workspace.resolve_existing(&input_path)?;
            let mut searcher = searcher
                .lock()
                .map_err(|_| anyhow::anyhow!("search index lock is poisoned"))?;
            let output = searcher.search(&options)?;
            Ok((workspace.display(&start), output.lines, output.metadata))
        })
        .await
        .map_err(|err| anyhow::anyhow!("grep search task failed: {err}"))?
    }

    async fn glob(&self, pattern: &str) -> anyhow::Result<Vec<String>> {
        let workspace = self.workspace.clone();
        let pattern = pattern.to_string();
        tokio::task::spawn_blocking(move || {
            let pattern = crate::search::normalize_relative_pattern(&pattern);
            let mut matches = Vec::new();
            crate::search::visit_files(workspace.root(), &mut |path| {
                let rel = workspace.display(path);
                if crate::search::wildcard_match(&pattern, &rel) {
                    matches.push(rel);
                }
                Ok(())
            })?;
            matches.sort();
            Ok(matches)
        })
        .await
        .map_err(|err| anyhow::anyhow!("glob task failed: {err}"))?
    }

    async fn apply_patch(&self, patch: &str) -> anyhow::Result<String> {
        let result = crate::patch::apply_patch_to_workspace(&self.workspace, patch).await?;
        self.invalidate_search_index()?;
        Ok(result)
    }
}

#[async_trait]
impl WorkspaceBackend for RunnerWorkspaceBackend {
    async fn read_text(&self, path: &str) -> anyhow::Result<(String, String)> {
        let path = self.guard.resolve_existing(path)?;
        let rel = self.guard.display(&path);
        let read = self
            .session
            .read_file(RunnerFileReadRequest {
                path: rel.clone().into(),
            })
            .await?;
        Ok((rel, String::from_utf8(read.contents)?))
    }

    async fn list_files(&self, path: &str) -> anyhow::Result<(String, Vec<String>)> {
        let path = self.guard.resolve_existing(path)?;
        let rel = self.guard.display(&path);
        // The workspace root displays as ""; quote "." so the glob stays inside the root.
        let quoted = shell_quote(if rel.is_empty() { "." } else { &rel });
        let command = format!(
            "for p in {quoted}/* {quoted}/.[!.]* {quoted}/..?*; do [ -e \"$p\" ] || continue; name=$(basename \"$p\"); if [ -d \"$p\" ]; then printf '%s/\\n' \"$name\"; else printf '%s\\n' \"$name\"; fi; done"
        );
        let output = self.run_shell(command).await?;
        let mut names = output.lines().map(ToString::to_string).collect::<Vec<_>>();
        names.sort();
        Ok((rel, names))
    }

    async fn write_text(&self, path: &str, content: String) -> anyhow::Result<String> {
        let path = self.guard.resolve_for_write(path)?;
        let rel = self.guard.display(&path);
        self.session
            .write_file(RunnerFileWriteRequest {
                path: rel.clone().into(),
                contents: content.into_bytes(),
            })
            .await?;
        Ok(rel)
    }

    /**
     * Routed through roder_edit_core like the local backend so an ambiguous
     * old_string is refused instead of silently rewriting the first
     * occurrence in the runner workspace (which the hosted sandbox
     * auto-commits).
     */
    async fn edit_text(
        &self,
        path: &str,
        old_string: &str,
        new_string: &str,
    ) -> anyhow::Result<Option<EditOutcome>> {
        let (rel, text) = self.read_text(path).await?;
        let (updated, outcome) = match roder_edit_core::apply_edit(
            rel.clone(),
            &text,
            old_string,
            new_string,
            roder_edit_core::EditOptions {
                fuzzy: roder_edit_core::EditMatchMode::Off,
                strip_line_numbers: false,
                reindent_inserted: false,
            },
        ) {
            Ok(result) => result,
            Err(_) => return Ok(None),
        };
        self.session
            .write_file(RunnerFileWriteRequest {
                path: rel.into(),
                contents: updated.into_bytes(),
            })
            .await?;
        Ok(Some(EditOutcome {
            path: outcome.path,
            replacements: outcome.replacements,
        }))
    }

    async fn multi_edit_text(
        &self,
        path: &str,
        edits: Vec<TextEdit>,
    ) -> anyhow::Result<Result<EditOutcome, usize>> {
        let (rel, text) = self.read_text(path).await?;
        let core_edits = edits
            .iter()
            .map(|edit| roder_edit_core::TextEdit {
                old_string: edit.old_string.clone(),
                new_string: edit.new_string.clone(),
            })
            .collect::<Vec<_>>();
        let (updated, outcome) = match roder_edit_core::apply_multi_edit(
            rel.clone(),
            &text,
            &core_edits,
            roder_edit_core::EditOptions {
                fuzzy: roder_edit_core::EditMatchMode::Off,
                strip_line_numbers: false,
                reindent_inserted: false,
            },
        ) {
            Ok(result) => result,
            Err(roder_edit_core::EditApplyError::OldStringNotFound { edit, .. }) => {
                return Ok(Err(edit.unwrap_or(0)));
            }
            Err(roder_edit_core::EditApplyError::OldStringAmbiguous { edit, .. }) => {
                return Ok(Err(edit.unwrap_or(0)));
            }
        };
        self.session
            .write_file(RunnerFileWriteRequest {
                path: rel.into(),
                contents: updated.into_bytes(),
            })
            .await?;
        Ok(Ok(EditOutcome {
            path: outcome.path,
            replacements: outcome.replacements,
        }))
    }

    /**
     * One shell round trip on the runner instead of a read_file per
     * candidate: every runner request is a full sandbox round trip, so the
     * per-file variant took minutes on real workspaces (node_modules
     * included) and starved the host's turn inactivity budget.
     */
    async fn grep_search(
        &self,
        options: SearchOptions,
    ) -> anyhow::Result<(String, Vec<String>, SearchMetadata)> {
        let started_at = std::time::Instant::now();
        let input_path = options.path.to_string_lossy().to_string();
        let start = self.guard.resolve_existing(&input_path)?;
        let start = self.guard.display(&start);
        /*
         * Validate regex queries locally first: the find pipeline below
         * absorbs grep's exit status (a no-match exit 1 is indistinguishable
         * from a bad pattern), so an invalid pattern must error here.
         */
        if options.regex {
            RegexBuilder::new(&options.query)
                .case_insensitive(!options.case_sensitive)
                .build()?;
        }
        let mut flags = String::from("-n");
        if !options.case_sensitive {
            flags.push('i');
        }
        if options.word_boundary {
            flags.push('w');
        }
        flags.push(if options.regex { 'E' } else { 'F' });
        let target = if start.is_empty() || start == "." {
            ".".to_string()
        } else {
            format!("./{start}")
        };
        /*
         * /dev/null forces the file:line: prefix even when a -exec batch
         * holds a single file; the sed strips find's ./ prefix so paths come
         * back workspace-relative. Regex queries run as ERE on the runner,
         * which can diverge from the local Rust regex engine on perl-style
         * classes like \d.
         */
        let command = format!(
            "find {} -type f ! -path './.git/*' ! -path './target/*' -exec grep {flags} -e {} /dev/null {{}} + 2>/dev/null | sed 's#^\\./##'",
            shell_quote(&target),
            shell_quote(&options.query),
        );
        let output = self.run_shell(command).await?;
        let matches = output.lines().map(ToString::to_string).collect::<Vec<_>>();
        let matched_files = matches
            .iter()
            .filter_map(|line| line.split(':').next())
            .collect::<std::collections::HashSet<_>>()
            .len();
        let metadata = SearchMetadata {
            engine: SearchEngine::Fallback,
            // The runner reports matches only, so files scanned is unknown.
            candidate_files: matched_files,
            verified_files: matched_files,
            stale: false,
            elapsed_ms: started_at.elapsed().as_millis(),
            index_version: INDEX_VERSION.to_string(),
            index_bytes: None,
            index_build_time_ms: None,
        };
        Ok((start, matches, metadata))
    }

    async fn glob(&self, pattern: &str) -> anyhow::Result<Vec<String>> {
        let pattern = crate::search::normalize_relative_pattern(pattern);
        let output = self
            .run_shell(
                "find . -type f ! -path './.git/*' ! -path './target/*' | sed 's#^./##'"
                    .to_string(),
            )
            .await?;
        let mut matches = output
            .lines()
            .filter(|rel| crate::search::wildcard_match(&pattern, rel))
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        matches.sort();
        Ok(matches)
    }

    async fn apply_patch(&self, patch: &str) -> anyhow::Result<String> {
        crate::patch::apply_patch_to_runner_workspace(&self.guard, self.session.as_ref(), patch)
            .await
    }
}

impl RunnerWorkspaceBackend {
    async fn run_shell(&self, command: String) -> anyhow::Result<String> {
        let output = self
            .session
            .run_command(RunnerCommandRequest {
                command_id: "workspace-backend".to_string(),
                program: "sh".to_string(),
                args: vec!["-lc".to_string(), command],
                cwd: Some(self.guard.root().to_path_buf()),
                env: Vec::new(),
            })
            .await?;
        if output.exit_code != Some(0) {
            anyhow::bail!("{}", output.stderr);
        }
        Ok(output.stdout)
    }
}

pub(crate) fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use roder_api::policy_mode::PolicyMode;
    use roder_api::remote_runner::RemoteWorkspace;

    use super::*;
    use crate::remote_test_support::{RecordingRunnerSession, RecordingRunnerState};

    fn remote_context(state: Arc<RecordingRunnerState>) -> ToolExecutionContext {
        ToolExecutionContext::new("thread-remote", "turn-remote", PolicyMode::Default)
            .with_remote_workspace(Arc::new(RemoteWorkspace {
                session: Arc::new(RecordingRunnerSession { state }),
                root: PathBuf::from("/sandbox/workspace"),
            }))
    }

    fn local_fallback(prefix: &str) -> (PathBuf, Workspace, WorkspaceBackendHandle) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("{prefix}-{nanos}"));
        std::fs::create_dir_all(&root).unwrap();
        let workspace = Workspace::new(root.clone()).unwrap();
        let backend: WorkspaceBackendHandle =
            Arc::new(LocalWorkspaceBackend::new(workspace.clone()));
        (root, workspace, backend)
    }

    #[tokio::test]
    async fn remote_context_routes_file_operations_through_runner_session() {
        let state = Arc::new(RecordingRunnerState::default());
        let ctx = remote_context(state.clone());
        let (root, fallback_workspace, fallback_backend) = local_fallback("runner-backend-files");

        let backend =
            backend_from_context_or_fallback(&ctx, &fallback_workspace, &fallback_backend).unwrap();
        backend
            .write_text("notes/todo.txt", "remote contents".to_string())
            .await
            .unwrap();
        let (path, text) = backend.read_text("notes/todo.txt").await.unwrap();

        assert_eq!(path, "notes/todo.txt");
        assert_eq!(text, "remote contents");
        assert_eq!(
            state
                .files
                .lock()
                .unwrap()
                .get("notes/todo.txt")
                .map(|contents| contents.as_slice()),
            Some(b"remote contents".as_slice())
        );
        // Nothing may leak onto the local fallback workspace.
        assert!(!root.join("notes").exists());
        assert!(!std::path::Path::new("/sandbox/workspace").exists());

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn remote_edit_matches_local_ambiguity_semantics() {
        let state = Arc::new(RecordingRunnerState::default());
        state
            .files
            .lock()
            .unwrap()
            .insert("notes/dup.txt".to_string(), b"same\nsame\n".to_vec());
        let ctx = remote_context(state.clone());
        let (root, fallback_workspace, fallback_backend) = local_fallback("runner-backend-edit");

        let backend =
            backend_from_context_or_fallback(&ctx, &fallback_workspace, &fallback_backend).unwrap();

        // Ambiguous old_string is refused; the runner file stays untouched.
        let ambiguous = backend
            .edit_text("notes/dup.txt", "same", "changed")
            .await
            .unwrap();
        assert_eq!(ambiguous, None);
        assert_eq!(
            state
                .files
                .lock()
                .unwrap()
                .get("notes/dup.txt")
                .map(|contents| contents.as_slice()),
            Some(b"same\nsame\n".as_slice())
        );
        let multi_ambiguous = backend
            .multi_edit_text(
                "notes/dup.txt",
                vec![TextEdit {
                    old_string: "same".to_string(),
                    new_string: "changed".to_string(),
                }],
            )
            .await
            .unwrap();
        assert_eq!(multi_ambiguous, Err(0));

        let unique = backend
            .edit_text("notes/dup.txt", "same\nsame", "one\ntwo")
            .await
            .unwrap();
        assert_eq!(
            unique,
            Some(EditOutcome {
                path: "notes/dup.txt".to_string(),
                replacements: 1,
            })
        );
        assert_eq!(
            state
                .files
                .lock()
                .unwrap()
                .get("notes/dup.txt")
                .map(|contents| contents.as_slice()),
            Some(b"one\ntwo\n".as_slice())
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn remote_grep_runs_a_single_server_side_search() {
        let state = Arc::new(RecordingRunnerState::default());
        let ctx = remote_context(state.clone());
        let (root, fallback_workspace, fallback_backend) = local_fallback("runner-backend-grep");

        let backend =
            backend_from_context_or_fallback(&ctx, &fallback_workspace, &fallback_backend).unwrap();
        let (start, matches, metadata) = backend
            .grep_search(SearchOptions::new("needle"))
            .await
            .unwrap();

        assert_eq!(start, "");
        // The recording session returns one canned stdout line per command.
        assert_eq!(matches, vec!["remote ok".to_string()]);
        assert!(metadata.elapsed_ms < 10_000);
        let commands = state.commands.lock().unwrap();
        assert_eq!(commands.len(), 1, "grep must be a single runner round trip");
        let script = commands[0].args.last().unwrap();
        assert!(script.contains("find '.' -type f"), "{script}");
        assert!(
            script.contains("grep -nF -e 'needle' /dev/null"),
            "{script}"
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn remote_context_applies_codex_patches_through_runner_session() {
        let state = Arc::new(RecordingRunnerState::default());
        state
            .files
            .lock()
            .unwrap()
            .insert("src/lib.rs".to_string(), b"old line\n".to_vec());
        let ctx = remote_context(state.clone());
        let (root, fallback_workspace, fallback_backend) = local_fallback("runner-backend-patch");

        let backend =
            backend_from_context_or_fallback(&ctx, &fallback_workspace, &fallback_backend).unwrap();
        let summary = backend
            .apply_patch(
                "*** Begin Patch\n*** Update File: src/lib.rs\n@@\n-old line\n+new line\n*** End Patch\n",
            )
            .await
            .unwrap();

        assert_eq!(summary, "Success. Updated src/lib.rs");
        assert_eq!(
            state
                .files
                .lock()
                .unwrap()
                .get("src/lib.rs")
                .map(|contents| contents.as_slice()),
            Some(b"new line\n".as_slice())
        );
        assert!(!root.join("src").exists());

        let _ = std::fs::remove_dir_all(root);
    }
}
