pub mod compute;
pub mod keys;
pub mod render;

use std::path::PathBuf;

use roder_api::events::FileChangePreviewReady;

use compute::{Hunk, HunkStatus, compute_diff};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum DiffViewMode {
    Unified,
    SideBySide,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum DiffResolution {
    Accepted,
    Rejected,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PendingDiff {
    pub call_id: String,
    pub tool: String,
    pub files: Vec<FileDiff>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct FileDiff {
    pub path: PathBuf,
    pub change_type: String,
    pub before: Option<String>,
    pub after: String,
    pub supports_partial: bool,
    pub hunks: Vec<Hunk>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DiffViewerState {
    pub pending: PendingDiff,
    pub file_index: usize,
    pub hunk_index: usize,
    pub mode: DiffViewMode,
    pub resolution: Option<DiffResolution>,
}

impl DiffViewerState {
    pub fn from_preview(preview: FileChangePreviewReady) -> Self {
        let hunks = compute_diff(preview.before.as_deref(), &preview.after);
        Self::new(PendingDiff {
            call_id: preview.tool_id,
            tool: preview.tool_name,
            files: vec![FileDiff {
                path: PathBuf::from(preview.path),
                change_type: preview.change_type,
                before: preview.before,
                after: preview.after,
                supports_partial: preview.supports_partial,
                hunks,
            }],
        })
    }

    pub fn new(pending: PendingDiff) -> Self {
        Self {
            pending,
            file_index: 0,
            hunk_index: 0,
            mode: DiffViewMode::Unified,
            resolution: None,
        }
    }

    pub fn current_file(&self) -> Option<&FileDiff> {
        self.pending.files.get(self.file_index)
    }

    pub fn current_file_mut(&mut self) -> Option<&mut FileDiff> {
        self.pending.files.get_mut(self.file_index)
    }

    pub fn current_hunk(&self) -> Option<&Hunk> {
        self.current_file()
            .and_then(|file| file.hunks.get(self.hunk_index))
    }

    pub fn current_hunk_mut(&mut self) -> Option<&mut Hunk> {
        let hunk_index = self.hunk_index;
        self.current_file_mut()
            .and_then(|file| file.hunks.get_mut(hunk_index))
    }

    pub fn hunk_count(&self) -> usize {
        self.current_file()
            .map(|file| file.hunks.len())
            .unwrap_or_default()
    }

    pub fn file_count(&self) -> usize {
        self.pending.files.len()
    }

    pub fn next_file(&mut self) {
        if self.file_count() > 0 {
            self.file_index = (self.file_index + 1).min(self.file_count() - 1);
            self.hunk_index = 0;
        }
    }

    pub fn previous_file(&mut self) {
        self.file_index = self.file_index.saturating_sub(1);
        self.hunk_index = 0;
    }

    pub fn supports_partial(&self) -> bool {
        self.current_file()
            .map(|file| file.supports_partial)
            .unwrap_or(false)
    }

    pub fn set_all_hunks(&mut self, status: HunkStatus) {
        for file in &mut self.pending.files {
            for hunk in &mut file.hunks {
                hunk.status = status;
            }
        }
    }

    pub fn clamp_cursor(&mut self) {
        let file_count = self.pending.files.len();
        if file_count == 0 {
            self.file_index = 0;
            self.hunk_index = 0;
            return;
        }
        self.file_index = self.file_index.min(file_count - 1);
        let hunk_count = self.hunk_count();
        if hunk_count == 0 {
            self.hunk_index = 0;
        } else {
            self.hunk_index = self.hunk_index.min(hunk_count - 1);
        }
    }
}
