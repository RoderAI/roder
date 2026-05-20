use std::sync::{Arc, Mutex};

use async_trait::async_trait;
#[cfg(test)]
use regex::RegexBuilder;
#[cfg(test)]
use roder_api::remote_runner::{
    RemoteRunnerSession, RunnerCommandRequest, RunnerFileReadRequest, RunnerFileWriteRequest,
};
use roder_api::tools::ToolExecutionContext;
#[cfg(test)]
use roder_search::{INDEX_VERSION, SearchEngine};
use roder_search::{SearchMetadata, SearchOptions};

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
    searcher: Mutex<roder_search::WorkspaceSearcher>,
}

#[cfg(test)]
pub(crate) struct RunnerWorkspaceBackend {
    guard: Workspace,
    session: Arc<dyn RemoteRunnerSession>,
}

#[cfg(test)]
impl RunnerWorkspaceBackend {
    pub(crate) fn new(guard: Workspace, session: Arc<dyn RemoteRunnerSession>) -> Self {
        Self { guard, session }
    }
}

impl LocalWorkspaceBackend {
    pub(crate) fn new(workspace: Workspace) -> Self {
        let searcher = Mutex::new(roder_search::WorkspaceSearcher::new(workspace.root()));
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
        let Some(index) = text.find(old_string) else {
            return Ok(None);
        };
        let mut updated = text;
        updated.replace_range(index..index + old_string.len(), new_string);
        std::fs::write(&path, updated)?;
        self.invalidate_search_index()?;
        Ok(Some(EditOutcome {
            path: self.workspace.display(&path),
            replacements: 1,
        }))
    }

    async fn multi_edit_text(
        &self,
        path: &str,
        edits: Vec<TextEdit>,
    ) -> anyhow::Result<Result<EditOutcome, usize>> {
        let path = self.workspace.resolve_existing(path)?;
        let mut text = std::fs::read_to_string(&path)?;
        for (index, edit) in edits.iter().enumerate() {
            let Some(position) = text.find(&edit.old_string) else {
                return Ok(Err(index));
            };
            text.replace_range(position..position + edit.old_string.len(), &edit.new_string);
        }
        std::fs::write(&path, text)?;
        self.invalidate_search_index()?;
        Ok(Ok(EditOutcome {
            path: self.workspace.display(&path),
            replacements: edits.len(),
        }))
    }

    async fn grep_search(
        &self,
        options: SearchOptions,
    ) -> anyhow::Result<(String, Vec<String>, SearchMetadata)> {
        let input_path = options.path.to_string_lossy().to_string();
        let start = self.workspace.resolve_existing(&input_path)?;
        let mut searcher = self
            .searcher
            .lock()
            .map_err(|_| anyhow::anyhow!("search index lock is poisoned"))?;
        let output = searcher.search(&options)?;
        Ok((
            self.workspace.display(&start),
            output.lines,
            output.metadata,
        ))
    }

    async fn glob(&self, pattern: &str) -> anyhow::Result<Vec<String>> {
        let mut matches = Vec::new();
        crate::search::visit_files(self.workspace.root(), &mut |path| {
            let rel = self.workspace.display(path);
            if crate::search::wildcard_match(pattern, &rel) {
                matches.push(rel);
            }
            Ok(())
        })?;
        matches.sort();
        Ok(matches)
    }

    async fn apply_patch(&self, patch: &str) -> anyhow::Result<String> {
        let result = crate::patch::apply_patch_to_workspace(&self.workspace, patch).await?;
        self.invalidate_search_index()?;
        Ok(result)
    }
}

#[cfg(test)]
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
        let quoted = shell_quote(&rel);
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

    async fn edit_text(
        &self,
        path: &str,
        old_string: &str,
        new_string: &str,
    ) -> anyhow::Result<Option<EditOutcome>> {
        let (rel, text) = self.read_text(path).await?;
        let Some(index) = text.find(old_string) else {
            return Ok(None);
        };
        let mut updated = text;
        updated.replace_range(index..index + old_string.len(), new_string);
        self.session
            .write_file(RunnerFileWriteRequest {
                path: rel.clone().into(),
                contents: updated.into_bytes(),
            })
            .await?;
        Ok(Some(EditOutcome {
            path: rel,
            replacements: 1,
        }))
    }

    async fn multi_edit_text(
        &self,
        path: &str,
        edits: Vec<TextEdit>,
    ) -> anyhow::Result<Result<EditOutcome, usize>> {
        let (rel, mut text) = self.read_text(path).await?;
        for (index, edit) in edits.iter().enumerate() {
            let Some(position) = text.find(&edit.old_string) else {
                return Ok(Err(index));
            };
            text.replace_range(position..position + edit.old_string.len(), &edit.new_string);
        }
        self.session
            .write_file(RunnerFileWriteRequest {
                path: rel.clone().into(),
                contents: text.into_bytes(),
            })
            .await?;
        Ok(Ok(EditOutcome {
            path: rel,
            replacements: edits.len(),
        }))
    }

    async fn grep_search(
        &self,
        options: SearchOptions,
    ) -> anyhow::Result<(String, Vec<String>, SearchMetadata)> {
        let started_at = std::time::Instant::now();
        let input_path = options.path.to_string_lossy().to_string();
        let start = self.guard.resolve_existing(&input_path)?;
        let start = self.guard.display(&start);
        let files = self.glob("*").await?;
        let mut matches = Vec::new();
        let mut verified_files = 0;
        let pattern = if options.regex {
            options.query.clone()
        } else {
            regex::escape(&options.query)
        };
        let pattern = if options.word_boundary {
            format!(r"\b(?:{})\b", pattern)
        } else {
            pattern
        };
        let matcher = RegexBuilder::new(&pattern)
            .case_insensitive(!options.case_sensitive)
            .build()?;
        for file in files.into_iter().filter(|file| {
            start.is_empty()
                || start == "."
                || file == &start
                || file
                    .strip_prefix(&start)
                    .is_some_and(|s| s.starts_with('/'))
        }) {
            let Ok((_, text)) = self.read_text(&file).await else {
                continue;
            };
            verified_files += 1;
            for (line_index, line) in text.lines().enumerate() {
                if matcher.is_match(line) {
                    matches.push(format!("{file}:{}:{line}", line_index + 1));
                }
            }
        }
        let metadata = SearchMetadata {
            engine: SearchEngine::Fallback,
            candidate_files: verified_files,
            verified_files,
            stale: false,
            elapsed_ms: started_at.elapsed().as_millis(),
            index_version: INDEX_VERSION.to_string(),
            index_bytes: None,
            index_build_time_ms: None,
        };
        Ok((start, matches, metadata))
    }

    async fn glob(&self, pattern: &str) -> anyhow::Result<Vec<String>> {
        let output = self
            .run_shell(
                "find . -type f ! -path './.git/*' ! -path './target/*' | sed 's#^./##'"
                    .to_string(),
            )
            .await?;
        let mut matches = output
            .lines()
            .filter(|rel| crate::search::wildcard_match(pattern, rel))
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        matches.sort();
        Ok(matches)
    }

    async fn apply_patch(&self, patch: &str) -> anyhow::Result<String> {
        crate::patch::apply_patch_to_workspace(&self.guard, patch).await
    }
}

#[cfg(test)]
impl RunnerWorkspaceBackend {
    async fn run_shell(&self, command: String) -> anyhow::Result<String> {
        let output = self
            .session
            .run_command(RunnerCommandRequest {
                command_id: "workspace-backend".to_string(),
                program: "sh".to_string(),
                args: vec!["-lc".to_string(), command],
                cwd: None,
                env: Vec::new(),
            })
            .await?;
        if output.exit_code != Some(0) {
            anyhow::bail!("{}", output.stderr);
        }
        Ok(output.stdout)
    }
}

#[cfg(test)]
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
