//! Image-owned layout, asynchronous preparation, and clipped rendering.

use std::{
    cell::RefCell,
    collections::BTreeMap,
    path::PathBuf,
    sync::mpsc::{self, Receiver, SyncSender},
};

use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
};
use ratatui_image::sliced::SignedPosition;

use super::{
    super::Theme,
    image::{self, LoadedImage, TerminalImages},
};

#[cfg(test)]
#[path = "preview_worker_tests.rs"]
mod worker_tests;

#[path = "preview_suspension.rs"]
mod suspension;

const MAX_PREVIEWS: usize = 32;
const MAX_PER_ITEM: usize = 4;
const WORK_QUEUE: usize = 4;

struct Entry {
    id: u64,
    path: PathBuf,
    width: u16,
    pending: bool,
    image: Option<LoadedImage>,
    error: Option<String>,
}

impl Entry {
    fn rows(&self) -> usize {
        1 + self
            .image
            .as_ref()
            .map_or(0, |image| usize::from(image.size.height))
    }
}

struct Job {
    id: u64,
    path: PathBuf,
    width: u16,
}

struct Completion {
    id: u64,
    width: u16,
    image: Result<LoadedImage, String>,
}

struct Worker {
    jobs: SyncSender<Job>,
    results: Receiver<Completion>,
}

impl Worker {
    fn new(terminal: TerminalImages) -> std::io::Result<Self> {
        let (jobs, receiver) = mpsc::sync_channel::<Job>(WORK_QUEUE);
        let (sender, results) = mpsc::sync_channel(WORK_QUEUE);
        std::thread::Builder::new()
            .name("transcript-images".into())
            .spawn(move || {
                while let Ok(job) = receiver.recv() {
                    let image = image::load(&job.path, job.width, terminal);
                    if sender
                        .send(Completion {
                            id: job.id,
                            width: job.width,
                            image,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })?;
        Ok(Self { jobs, results })
    }
}

pub(in crate::app) struct ImagePreviews {
    entries: BTreeMap<usize, Vec<Entry>>,
    terminal: Option<TerminalImages>,
    worker: Option<Worker>,
    next_id: u64,
    visible: bool,
    last_sixel_areas: RefCell<Vec<Rect>>,
}

impl std::fmt::Debug for ImagePreviews {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImagePreviews")
            .field("items", &self.entries.len())
            .field("terminal", &self.terminal)
            .finish_non_exhaustive()
    }
}

impl Default for ImagePreviews {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
            terminal: TerminalImages::detect(),
            worker: None,
            next_id: 0,
            visible: true,
            last_sixel_areas: RefCell::new(Vec::new()),
        }
    }
}

impl ImagePreviews {
    pub fn attach(&mut self, item: usize, paths: impl IntoIterator<Item = PathBuf>) {
        let paths = paths.into_iter().take(MAX_PER_ITEM).collect::<Vec<_>>();
        if paths.is_empty() {
            return;
        }
        let entries = self.entries.entry(item).or_default();
        for path in paths {
            if entries.len() >= MAX_PER_ITEM {
                break;
            }
            if entries.iter().any(|entry| entry.path == path) {
                continue;
            }
            self.next_id += 1;
            entries.push(Entry {
                id: self.next_id,
                path,
                width: 0,
                pending: false,
                image: None,
                error: self
                    .terminal
                    .is_none()
                    .then(|| "open file to view; terminal preview unavailable".into()),
            });
        }
        while self.entries.values().map(Vec::len).sum::<usize>() > MAX_PREVIEWS {
            self.entries.pop_first();
        }
    }

    /// Called from the update loop, never from rendering. File I/O, decoding, resizing and
    /// protocol encoding happen on one worker with bounded job/result queues.
    pub fn update(&mut self, width: u16) -> bool {
        let mut changed = false;
        let mut disconnected = false;
        if let Some(worker) = &self.worker {
            loop {
                let completion = match worker.results.try_recv() {
                    Ok(completion) => completion,
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                };
                if let Some(entry) = self
                    .entries
                    .values_mut()
                    .flatten()
                    .find(|entry| entry.id == completion.id)
                {
                    entry.pending = false;
                    if completion.width == width {
                        match completion.image {
                            Ok(image) => {
                                entry.image = Some(image);
                                entry.error = None;
                            }
                            Err(error) => entry.error = Some(error),
                        }
                        changed = true;
                    }
                }
            }
        }
        if disconnected {
            for entry in self.entries.values_mut().flatten() {
                entry.pending = false;
                entry.error = Some("preview worker stopped; open file to view".into());
            }
            self.worker = None;
            self.terminal = None;
            return true;
        }
        let Some(terminal) = self.terminal else {
            return changed;
        };
        if !self.visible || width < 3 || self.entries.is_empty() {
            return changed;
        }
        if self.worker.is_none() {
            match Worker::new(terminal) {
                Ok(worker) => self.worker = Some(worker),
                Err(_) => {
                    for entry in self.entries.values_mut().flatten() {
                        entry.error = Some("preview worker unavailable; open file to view".into());
                    }
                    self.terminal = None;
                    return true;
                }
            }
        }
        let Some(worker) = self.worker.as_ref() else {
            return changed;
        };
        for entry in self.entries.values_mut().flatten() {
            if entry.pending || entry.width == width {
                continue;
            }
            let job = Job {
                id: entry.id,
                path: entry.path.clone(),
                width,
            };
            if worker.jobs.try_send(job).is_err() {
                break;
            }
            // Do not paint an old wider image while preparing a smaller terminal layout.
            if entry.image.take().is_some() {
                changed = true;
            }
            entry.width = width;
            entry.pending = true;
            entry.error = None;
        }
        changed
    }

    pub fn prepend(&mut self) {
        self.entries = std::mem::take(&mut self.entries)
            .into_iter()
            .map(|(index, entries)| (index + 1, entries))
            .collect();
    }

    pub fn remove(&mut self, removed: usize) {
        self.entries.remove(&removed);
        self.entries = std::mem::take(&mut self.entries)
            .into_iter()
            .map(|(index, entries)| (if index > removed { index - 1 } else { index }, entries))
            .collect();
    }

    pub fn append_rows(
        &self,
        item: usize,
        theme: Theme,
        width: u16,
        lines: &mut Vec<Line<'static>>,
    ) {
        if let Some(entries) = self.entries.get(&item) {
            for entry in entries {
                let status = entry.error.as_deref().unwrap_or(if entry.image.is_some() {
                    "preview"
                } else {
                    "loading preview"
                });
                // File names are untrusted text; control characters cannot reach the terminal.
                let label: String = entry
                    .path
                    .to_string_lossy()
                    .chars()
                    .map(|c| if c.is_control() { ' ' } else { c })
                    .collect();
                // `.timeline-image` inherits the transcript's semantic muted text style.
                let label = truncate_cells(&format!("  image ({status}): {label}"), width);
                lines.push(Line::from(Span::styled(label, theme.muted())));
                if let Some(image) = &entry.image {
                    lines.extend((0..image.size.height).map(|_| Line::raw("")));
                }
            }
        }
    }

    pub fn render(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        scroll: usize,
        row_items: &[(usize, usize)],
    ) {
        self.last_sixel_areas.borrow_mut().clear();
        if !self.visible {
            return;
        }
        for (item, entries) in &self.entries {
            let Some(last_row) = row_items
                .iter()
                .rev()
                .find_map(|(row, index)| (index == item).then_some(*row))
            else {
                continue;
            };
            let reserved_rows = entries.iter().map(Entry::rows).sum::<usize>();
            let mut row = last_row + 1 - reserved_rows;
            for entry in entries {
                row += 1; // The human-readable metadata label is not image-owned.
                if let Some(image) = &entry.image {
                    if let Some(position) =
                        visible_position(row, image.size.height, scroll, area.height)
                    {
                        if image.size.width <= area.width.saturating_sub(2) {
                            if image.is_sixel() {
                                let y = position.y.max(0) as u16;
                                let skipped = position.y.min(0).unsigned_abs();
                                let height = image
                                    .size
                                    .height
                                    .saturating_sub(skipped)
                                    .min(area.height.saturating_sub(y));
                                self.last_sixel_areas.borrow_mut().push(Rect::new(
                                    area.x + 2,
                                    area.y + y,
                                    image.size.width,
                                    height,
                                ));
                            }
                            image.render(frame, area, position);
                        }
                    }
                    row += usize::from(image.size.height);
                }
            }
        }
    }
}

fn visible_position(row: usize, rows: u16, scroll: usize, height: u16) -> Option<SignedPosition> {
    let relative = row as i64 - scroll as i64;
    if relative < i64::from(height)
        && relative + i64::from(rows) > 0
        && relative + i64::from(rows) <= i64::from(i16::MAX)
    {
        Some(SignedPosition::from((2, i16::try_from(relative).ok()?)))
    } else {
        None
    }
}

fn truncate_cells(text: &str, width: u16) -> String {
    let mut used = 0;
    text.chars()
        .take_while(|character| {
            used += Span::raw(character.to_string()).width();
            used <= usize::from(width)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_placements_clip_both_edges_of_the_transcript() {
        assert!(visible_position(10, 5, 0, 10).is_none());
        assert!(visible_position(0, 5, 5, 10).is_none());
        let clipped_top = visible_position(0, 5, 3, 10).unwrap();
        assert_eq!(clipped_top.y, -3);
        assert_eq!(clipped_top.x, 2);
        assert_eq!(visible_position(8, 5, 0, 10).unwrap().y, 8);
    }

    #[test]
    fn fallback_rows_are_sanitized_and_follow_item_indices() {
        let mut previews = ImagePreviews {
            terminal: None,
            ..ImagePreviews::default()
        };
        previews.attach(1, [PathBuf::from("/tmp/picture\x1b.png")]);
        previews.prepend();
        let mut lines = Vec::new();
        previews.append_rows(2, Theme::for_terminal(), 100, &mut lines);
        assert_eq!(lines.len(), 1);
        let text = lines[0].to_string();
        assert!(text.contains("open file to view"));
        assert!(!text.contains('\x1b'));
        previews.remove(0);
        assert!(previews.entries.contains_key(&1));
        previews.remove(1);
        assert!(previews.entries.is_empty());
    }

    #[test]
    fn preview_retention_is_bounded_and_duplicate_paths_are_not_reloaded() {
        let mut previews = ImagePreviews {
            terminal: None,
            ..ImagePreviews::default()
        };
        for index in 0..100 {
            previews.attach(index, [PathBuf::from("a.png"), PathBuf::from("a.png")]);
        }
        assert_eq!(
            previews.entries.values().map(Vec::len).sum::<usize>(),
            MAX_PREVIEWS
        );
    }

    #[test]
    fn long_and_wide_metadata_never_creates_unreserved_wrap_rows() {
        let mut previews = ImagePreviews {
            terminal: None,
            ..ImagePreviews::default()
        };
        previews.attach(
            0,
            [PathBuf::from(format!(
                "/tmp/{}.png",
                "\u{753b}".repeat(100)
            ))],
        );
        let mut lines = Vec::new();
        previews.append_rows(0, Theme::for_terminal(), 15, &mut lines);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].width() <= 15);
    }

    #[test]
    fn image_rows_do_not_enter_text_and_never_paint_outside_transcript() {
        use ratatui::{Terminal, backend::TestBackend, widgets::Paragraph};
        use ratatui_image::{FontSize, picker::ProtocolType};

        let path =
            std::env::temp_dir().join(format!("roder-preview-layout-{}.png", std::process::id()));
        ::image::RgbaImage::from_pixel(16, 128, ::image::Rgba([30, 40, 50, 255]))
            .save(&path)
            .unwrap();
        for protocol in [ProtocolType::Kitty, ProtocolType::Sixel] {
            let terminal_images = TerminalImages {
                protocol,
                cell: FontSize::new(8, 16),
                tmux: false,
            };
            let mut previews = ImagePreviews {
                terminal: Some(terminal_images),
                ..ImagePreviews::default()
            };
            previews.attach(0, [path.clone()]);
            previews.entries.get_mut(&0).unwrap()[0].image =
                Some(image::load(&path, 20, terminal_images).unwrap());
            let mut lines = Vec::new();
            previews.append_rows(0, Theme::for_terminal(), 20, &mut lines);
            assert_eq!(lines.len(), 9); // One label plus eight reserved rows.
            assert!(lines.iter().all(|line| !line.to_string().contains('\x1b')));
            assert!(lines.iter().skip(1).all(|line| line.to_string().is_empty()));

            let row_items = (0..9).map(|row| (row, 0)).collect::<Vec<_>>();
            let mut terminal = Terminal::new(TestBackend::new(20, 8)).unwrap();
            terminal
                .draw(|frame| {
                    frame.render_widget(Paragraph::new("composer"), Rect::new(0, 5, 20, 1));
                    previews.render(frame, Rect::new(0, 0, 20, 5), 3, &row_items);
                })
                .unwrap();
            let buffer = terminal.backend().buffer();
            assert_eq!(buffer[(0, 5)].symbol(), "c");
            for y in 5..8 {
                for x in 0..20 {
                    assert!(!buffer[(x, y)].symbol().contains('\x1b'));
                }
            }
            terminal
                .draw(|frame| {
                    frame.render_widget(Paragraph::new("composer"), Rect::new(0, 5, 20, 1));
                })
                .unwrap();
            assert!(
                terminal
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .all(|cell| !cell.symbol().contains('\x1b'))
            );
        }
        std::fs::remove_file(path).unwrap();
    }
}
