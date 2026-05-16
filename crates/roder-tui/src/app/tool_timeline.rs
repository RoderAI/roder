mod markdown;
mod patch_preview;
mod preview;
mod render;
#[cfg(test)]
mod tests;

use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    layout::Rect,
    style::Modifier,
    text::{Line, Span, Text},
};

use super::{Theme, short_id};
use preview::{argument_preview, tool_title};
use render::{max_scroll, visible_hit_rows};

const BOTTOM_PADDING_ROWS: usize = 3;

#[derive(Debug, Clone, Eq, PartialEq)]
pub(super) struct ToolTimelineEntry {
    pub name: String,
    pub arguments: String,
}

impl ToolTimelineEntry {
    pub fn new(name: impl Into<String>, arguments: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            arguments: arguments.into(),
        }
    }

    pub fn label(&self) -> String {
        let title = tool_title(&self.name);
        let arguments = argument_preview(&self.arguments);
        if arguments.is_empty() {
            title
        } else {
            format!("{title} {arguments}")
        }
    }
}

pub(super) fn fallback_entry(name: impl Into<String>) -> ToolTimelineEntry {
    ToolTimelineEntry::new(name, "")
}

#[derive(Debug, Clone, Eq, PartialEq)]
enum TimelineItemKind {
    User(String),
    Assistant { text: String, phase: Option<String> },
    Reasoning(String),
    System(String),
    TurnCompleted(TurnCompletedSummary),
    Error(String),
    Shell(String),
    ShellOutput(String),
    Tool(ToolTimelineTool),
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(super) struct TurnCompletedSummary {
    pub elapsed: Duration,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub session_tokens: u64,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct TimelineItem {
    kind: TimelineItemKind,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct ToolTimelineTool {
    tool_id: String,
    entry: ToolTimelineEntry,
    status: ToolTimelineStatus,
    output: Option<String>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum ToolTimelineStatus {
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub(super) enum TimelineFocus {
    #[default]
    Composer,
    Timeline,
}

pub(super) struct TimelineRender {
    pub text: Text<'static>,
    pub scroll: u16,
}

#[derive(Debug, Default)]
pub(super) struct TimelineState {
    items: Vec<TimelineItem>,
    tool_indices: HashMap<String, usize>,
    selected: Option<usize>,
    expanded: HashSet<usize>,
    focus: TimelineFocus,
    auto_follow: bool,
    scroll_offset: usize,
    hit_rows: Vec<(u16, usize)>,
}

impl TimelineState {
    pub fn focus(&self) -> TimelineFocus {
        self.focus
    }

    pub fn is_focused(&self) -> bool {
        self.focus == TimelineFocus::Timeline
    }

    pub fn focus_latest(&mut self) {
        self.focus = TimelineFocus::Timeline;
        self.selected = self.selectable_indices().last().copied();
        self.auto_follow = self
            .selected
            .is_none_or(|index| index + 1 == self.items.len());
    }

    pub fn focus_composer(&mut self) {
        self.focus = TimelineFocus::Composer;
        self.selected = None;
        self.auto_follow = true;
    }

    pub fn push_user(&mut self, text: impl Into<String>) {
        self.push_item(TimelineItemKind::User(text.into()));
    }

    pub fn push_assistant_delta(&mut self, text: &str, phase: Option<String>) {
        if let Some(TimelineItem {
            kind:
                TimelineItemKind::Assistant {
                    text: existing,
                    phase: existing_phase,
                },
        }) = self.items.last_mut()
            && *existing_phase == phase
        {
            existing.push_str(text);
            self.follow_live_updates_from_composer();
            return;
        }
        self.push_item(TimelineItemKind::Assistant {
            text: text.to_string(),
            phase,
        });
    }

    pub fn push_reasoning_delta(&mut self, text: &str) {
        if let Some(TimelineItem {
            kind: TimelineItemKind::Reasoning(existing),
        }) = self.items.last_mut()
        {
            existing.push_str(text);
            self.follow_live_updates_from_composer();
            return;
        }
        self.push_item(TimelineItemKind::Reasoning(text.to_string()));
    }

    pub fn push_system(&mut self, text: impl Into<String>) {
        self.push_item(TimelineItemKind::System(text.into()));
    }

    pub fn push_error(&mut self, text: impl Into<String>) {
        self.push_item(TimelineItemKind::Error(text.into()));
    }

    pub fn push_shell(&mut self, command: impl Into<String>) {
        self.push_item(TimelineItemKind::Shell(command.into()));
    }

    pub fn push_shell_output(&mut self, output: impl Into<String>) {
        self.push_item(TimelineItemKind::ShellOutput(output.into()));
    }

    pub fn record_tool_requested(&mut self, tool_id: String, entry: ToolTimelineEntry) {
        if let Some(index) = self.tool_indices.get(&tool_id).copied() {
            if let Some(TimelineItem {
                kind: TimelineItemKind::Tool(tool),
            }) = self.items.get_mut(index)
            {
                if !entry.name.trim().is_empty() {
                    tool.entry.name = entry.name;
                }
                if !entry.arguments.trim().is_empty() {
                    tool.entry.arguments = entry.arguments;
                }
            }
            return;
        }
        let index = self.items.len();
        self.items.push(TimelineItem {
            kind: TimelineItemKind::Tool(ToolTimelineTool {
                tool_id: tool_id.clone(),
                entry,
                status: ToolTimelineStatus::Running,
                output: None,
            }),
        });
        self.tool_indices.insert(tool_id, index);
        self.follow_live_updates_from_composer();
    }

    pub fn record_tool_delta(&mut self, tool_id: &str, arguments_delta: &str) {
        let index = if let Some(index) = self.tool_indices.get(tool_id).copied() {
            index
        } else {
            let entry = fallback_entry(format!("tool {}", short_id(tool_id)));
            self.record_tool_requested(tool_id.to_string(), entry);
            self.tool_indices.get(tool_id).copied().unwrap_or(0)
        };
        if let Some(TimelineItem {
            kind: TimelineItemKind::Tool(tool),
        }) = self.items.get_mut(index)
        {
            tool.entry.arguments.push_str(arguments_delta);
            self.follow_live_updates_from_composer();
        }
    }

    pub fn record_tool_completed(&mut self, tool_id: &str, failed: bool, output: Option<String>) {
        let index = if let Some(index) = self.tool_indices.get(tool_id).copied() {
            index
        } else {
            let entry = fallback_entry(format!("tool {}", short_id(tool_id)));
            self.record_tool_requested(tool_id.to_string(), entry);
            self.tool_indices.get(tool_id).copied().unwrap_or(0)
        };
        if let Some(TimelineItem {
            kind: TimelineItemKind::Tool(tool),
        }) = self.items.get_mut(index)
        {
            tool.status = if failed {
                ToolTimelineStatus::Failed
            } else {
                ToolTimelineStatus::Completed
            };
            tool.output = output.filter(|text| !text.trim().is_empty());
            self.follow_live_updates_from_composer();
        }
    }

    pub fn push_turn_completed(&mut self, summary: TurnCompletedSummary) {
        self.push_item(TimelineItemKind::TurnCompleted(summary));
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::SHIFT) {
            return false;
        }

        match key.code {
            KeyCode::Esc => {
                self.focus_composer();
                true
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.select_next();
                true
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.select_previous();
                true
            }
            KeyCode::PageDown => {
                self.scroll_by(8);
                true
            }
            KeyCode::PageUp => {
                self.scroll_by(-8);
                true
            }
            KeyCode::Home => {
                self.scroll_to(0);
                true
            }
            KeyCode::End => {
                self.auto_follow = true;
                self.selected = self.selectable_indices().last().copied();
                true
            }
            KeyCode::Enter => {
                self.toggle_selected_expansion();
                true
            }
            _ => false,
        }
    }

    pub fn handle_mouse(&mut self, event: MouseEvent) -> bool {
        match event.kind {
            MouseEventKind::ScrollDown => {
                self.focus = TimelineFocus::Timeline;
                self.scroll_by(3);
                return true;
            }
            MouseEventKind::ScrollUp => {
                self.focus = TimelineFocus::Timeline;
                self.scroll_by(-3);
                return true;
            }
            MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left) => {}
            _ => return false,
        }
        let Some((_, index)) = self
            .hit_rows
            .iter()
            .find(|(row, _)| *row == event.row)
            .copied()
        else {
            return false;
        };
        let was_selected = self.selected == Some(index);
        self.focus = TimelineFocus::Timeline;
        self.selected = Some(index);
        self.auto_follow = index + 1 == self.items.len();
        if was_selected && self.item_can_expand(index) {
            self.toggle_expansion(index);
        }
        true
    }

    pub fn render(&mut self, theme: Theme, area: Rect) -> TimelineRender {
        let (lines, row_items) = self.build_lines(theme, area.width);
        let max_scroll = max_scroll(lines.len(), area.height);
        let scroll = self.scroll_for(area.height, &row_items, max_scroll);
        self.hit_rows = visible_hit_rows(area, scroll, area.height, &row_items);
        self.scroll_offset = usize::from(scroll);
        TimelineRender {
            text: Text::from(lines),
            scroll,
        }
    }

    fn push_item(&mut self, kind: TimelineItemKind) {
        self.items.push(TimelineItem { kind });
        self.follow_live_updates_from_composer();
    }

    fn follow_live_updates_from_composer(&mut self) {
        if self.focus == TimelineFocus::Composer {
            self.auto_follow = true;
        }
    }

    fn selectable_indices(&self) -> Vec<usize> {
        (0..self.items.len()).collect()
    }

    fn select_next(&mut self) {
        let selectable = self.selectable_indices();
        if selectable.is_empty() {
            self.selected = None;
            return;
        }
        let next = match self.selected {
            Some(current) => selectable
                .iter()
                .position(|index| *index == current)
                .map(|position| (position + 1).min(selectable.len() - 1))
                .unwrap_or(selectable.len() - 1),
            None => selectable.len() - 1,
        };
        self.selected = Some(selectable[next]);
        self.auto_follow = self.selected == selectable.last().copied();
    }

    fn select_previous(&mut self) {
        let selectable = self.selectable_indices();
        if selectable.is_empty() {
            self.selected = None;
            return;
        }
        let next = match self.selected {
            Some(current) => selectable
                .iter()
                .position(|index| *index == current)
                .map(|position| position.saturating_sub(1))
                .unwrap_or(selectable.len() - 1),
            None => selectable.len() - 1,
        };
        self.selected = Some(selectable[next]);
        self.auto_follow = false;
    }

    fn scroll_by(&mut self, amount: isize) {
        let current = self.scroll_offset as isize;
        self.scroll_offset = current.saturating_add(amount).max(0) as usize;
        self.selected = None;
        self.auto_follow = false;
    }

    fn scroll_to(&mut self, offset: usize) {
        self.scroll_offset = offset;
        self.selected = None;
        self.auto_follow = false;
    }

    fn toggle_selected_expansion(&mut self) {
        let Some(index) = self.selected else {
            return;
        };
        if self.item_can_expand(index) {
            self.toggle_expansion(index);
        }
    }

    fn toggle_expansion(&mut self, index: usize) {
        if !self.expanded.insert(index) {
            self.expanded.remove(&index);
        }
    }

    fn item_can_expand(&self, index: usize) -> bool {
        matches!(
            self.items.get(index),
            Some(TimelineItem {
                kind: TimelineItemKind::Tool(ToolTimelineTool {
                    output: Some(output),
                    ..
                }),
            }) if !output.trim().is_empty()
        )
    }

    fn scroll_for(&self, height: u16, row_items: &[(usize, usize)], max_scroll: usize) -> u16 {
        if row_items.is_empty() || height == 0 {
            return 0;
        }
        if self.auto_follow {
            return max_scroll as u16;
        }
        if self.focus == TimelineFocus::Timeline
            && let Some(selected) = self.selected
            && let Some((row, _)) = row_items.iter().find(|(_, index)| *index == selected)
        {
            let half = usize::from(height) / 2;
            return row.saturating_sub(half).min(max_scroll) as u16;
        }
        self.scroll_offset.min(max_scroll) as u16
    }

    fn build_lines(&self, theme: Theme, width: u16) -> (Vec<Line<'static>>, Vec<(usize, usize)>) {
        if self.items.is_empty() {
            return (
                vec![
                    Line::raw(""),
                    Line::from(Span::styled(
                        "No transcript yet. Ask Roder to inspect, edit, or run something.",
                        theme.muted().add_modifier(Modifier::ITALIC),
                    )),
                ],
                Vec::new(),
            );
        }

        let mut lines = Vec::new();
        let mut row_items = Vec::new();
        for (index, item) in self.items.iter().enumerate() {
            row_items.push((lines.len(), index));
            let selected = self.focus == TimelineFocus::Timeline && self.selected == Some(index);
            let expanded = self.expanded.contains(&index);
            item.render(selected, expanded, theme, width, &mut lines);
            if index + 1 < self.items.len() {
                lines.push(Line::raw(""));
            }
        }
        lines.extend((0..BOTTOM_PADDING_ROWS).map(|_| Line::raw("")));
        (lines, row_items)
    }
}
