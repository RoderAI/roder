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
use serde_json::Value;

use super::{Theme, short_id};
use preview::{argument_preview, tool_title};
use render::{max_scroll, visible_hit_rows};

const BOTTOM_PADDING_ROWS: usize = 3;
const TOOL_COLLAPSE_LIMIT: usize = 6;
const TOOL_OVERFLOW_INDEX: usize = usize::MAX;
const MOUSE_SCROLL_ROWS: isize = 10;
const MIN_PAGE_SCROLL_ROWS: isize = 12;

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
    pub reasoning_tokens: Option<u32>,
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

#[derive(Debug, Clone, Eq, PartialEq)]
pub(super) struct ToolDetail {
    pub title: String,
    pub command: Option<String>,
    pub arguments: String,
    pub output: Option<String>,
    pub failed: bool,
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
    last_viewport_height: u16,
    hit_rows: Vec<(u16, usize)>,
    show_all_tools: bool,
    requested_detail: Option<ToolDetail>,
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
                self.scroll_by(self.page_scroll_rows());
                true
            }
            KeyCode::PageUp => {
                self.scroll_by(-self.page_scroll_rows());
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
                self.scroll_by(MOUSE_SCROLL_ROWS);
                return true;
            }
            MouseEventKind::ScrollUp => {
                self.focus = TimelineFocus::Timeline;
                self.scroll_by(-MOUSE_SCROLL_ROWS);
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
        if index == TOOL_OVERFLOW_INDEX {
            self.focus = TimelineFocus::Timeline;
            self.show_all_tools = true;
            self.auto_follow = false;
            return true;
        }
        if !self.items.get(index).is_some_and(item_is_selectable) {
            return false;
        }
        let was_selected = self.selected == Some(index);
        self.focus = TimelineFocus::Timeline;
        self.selected = Some(index);
        self.auto_follow = index + 1 == self.items.len();
        if let Some(detail) = self.detail_for_index(index) {
            self.requested_detail = Some(detail);
            return true;
        }
        if was_selected && self.item_can_expand(index) {
            self.toggle_expansion(index);
        }
        true
    }

    pub fn take_requested_detail(&mut self) -> Option<ToolDetail> {
        self.requested_detail.take()
    }

    #[cfg(test)]
    pub fn render(&mut self, theme: Theme, area: Rect) -> TimelineRender {
        self.render_with_frame(theme, area, 0)
    }

    pub fn render_with_frame(
        &mut self,
        theme: Theme,
        area: Rect,
        animation_frame: u64,
    ) -> TimelineRender {
        let (lines, row_items, visual_height) =
            self.build_lines(theme, area.width, animation_frame);
        self.last_viewport_height = area.height;
        let max_scroll = max_scroll(visual_height, area.height);
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
        self.items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| item_is_selectable(item).then_some(index))
            .collect()
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

    fn page_scroll_rows(&self) -> isize {
        (self.last_viewport_height as isize)
            .saturating_sub(1)
            .max(MIN_PAGE_SCROLL_ROWS)
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
        if index == TOOL_OVERFLOW_INDEX {
            return !self.show_all_tools && self.hidden_tool_count() > 0;
        }
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

    fn detail_for_index(&self, index: usize) -> Option<ToolDetail> {
        match self.items.get(index)? {
            TimelineItem {
                kind: TimelineItemKind::Tool(tool),
            } => tool.detail(),
            TimelineItem {
                kind: TimelineItemKind::Shell(command),
            } => Some(ToolDetail {
                title: "Shell".to_string(),
                command: Some(command.clone()),
                arguments: String::new(),
                output: self.shell_output_after(index),
                failed: false,
            }),
            _ => None,
        }
    }

    fn shell_output_after(&self, index: usize) -> Option<String> {
        self.items
            .iter()
            .skip(index + 1)
            .find_map(|item| match &item.kind {
                TimelineItemKind::ShellOutput(output) if !output.trim().is_empty() => {
                    Some(output.clone())
                }
                _ => None,
            })
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

    fn build_lines(
        &self,
        theme: Theme,
        width: u16,
        animation_frame: u64,
    ) -> (Vec<Line<'static>>, Vec<(usize, usize)>, usize) {
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
                2,
            );
        }

        let mut lines = Vec::new();
        let mut row_items = Vec::new();
        let mut visual_row = 0;
        let visible_tools = self.visible_tool_indices();
        let hidden_tool_count = self.hidden_tool_count();
        let overflow_insert_index = visible_tools.first().copied();
        for (index, item) in self.items.iter().enumerate() {
            if Some(index) == overflow_insert_index && hidden_tool_count > 0 {
                let line_start = lines.len();
                render::push_tool_overflow_line(
                    hidden_tool_count,
                    self.focus == TimelineFocus::Timeline
                        && self.selected == Some(TOOL_OVERFLOW_INDEX),
                    theme,
                    width,
                    &mut lines,
                );
                visual_row = map_rendered_lines(
                    &lines,
                    line_start,
                    visual_row,
                    width,
                    TOOL_OVERFLOW_INDEX,
                    &mut row_items,
                );
            }
            if matches!(item.kind, TimelineItemKind::Tool(_))
                && !self.show_all_tools
                && !visible_tools.contains(&index)
            {
                continue;
            }
            let line_start = lines.len();
            let selected = self.focus == TimelineFocus::Timeline
                && self.selected == Some(index)
                && item_is_selectable(item);
            let expanded = self.expanded.contains(&index);
            item.render(
                selected,
                expanded,
                theme,
                width,
                animation_frame,
                &mut lines,
            );
            visual_row =
                map_rendered_lines(&lines, line_start, visual_row, width, index, &mut row_items);
            if self.should_separate_visible_item(index, &visible_tools) {
                lines.push(Line::raw(""));
                visual_row += 1;
            }
        }
        for _ in 0..BOTTOM_PADDING_ROWS {
            lines.push(Line::raw(""));
            visual_row += 1;
        }
        (lines, row_items, visual_row)
    }

    fn visible_tool_indices(&self) -> Vec<usize> {
        let tools = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                matches!(item.kind, TimelineItemKind::Tool(_)).then_some(index)
            })
            .collect::<Vec<_>>();
        if self.show_all_tools || tools.len() <= TOOL_COLLAPSE_LIMIT {
            return tools;
        }
        tools[tools.len() - TOOL_COLLAPSE_LIMIT..].to_vec()
    }

    fn hidden_tool_count(&self) -> usize {
        if self.show_all_tools {
            return 0;
        }
        self.items
            .iter()
            .filter(|item| matches!(item.kind, TimelineItemKind::Tool(_)))
            .count()
            .saturating_sub(TOOL_COLLAPSE_LIMIT)
    }

    fn should_separate_visible_item(&self, index: usize, visible_tools: &[usize]) -> bool {
        let Some(next_index) = self.next_visible_index(index, visible_tools) else {
            return false;
        };
        should_separate_items(&self.items[index], &self.items[next_index])
    }

    fn next_visible_index(&self, index: usize, visible_tools: &[usize]) -> Option<usize> {
        self.items
            .iter()
            .enumerate()
            .skip(index + 1)
            .find_map(|(candidate, item)| {
                if matches!(item.kind, TimelineItemKind::Tool(_))
                    && !self.show_all_tools
                    && !visible_tools.contains(&candidate)
                {
                    return None;
                }
                Some(candidate)
            })
    }
}

impl ToolTimelineTool {
    fn detail(&self) -> Option<ToolDetail> {
        if !is_shell_like_tool(&self.entry.name) {
            return None;
        }

        let (command, arguments) = command_and_arguments(&self.entry.arguments);
        Some(ToolDetail {
            title: self.entry.label(),
            command,
            arguments,
            output: self.output.clone().filter(|text| !text.trim().is_empty()),
            failed: self.status == ToolTimelineStatus::Failed,
        })
    }
}

fn is_shell_like_tool(name: &str) -> bool {
    let normalized = name.to_ascii_lowercase();
    normalized == "shell"
        || normalized == "exec"
        || normalized == "command"
        || normalized.ends_with("_shell")
        || normalized.ends_with(".shell")
        || normalized.contains("shell_command")
        || normalized.contains("exec_command")
}

fn command_and_arguments(arguments: &str) -> (Option<String>, String) {
    let trimmed = arguments.trim();
    if trimmed.is_empty() || trimmed == "{}" {
        return (None, String::new());
    }

    let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
        return (Some(trimmed.to_string()), String::new());
    };

    let command = command_from_json(&value);
    let pretty_arguments =
        serde_json::to_string_pretty(&value).unwrap_or_else(|_| trimmed.to_string());
    (command, pretty_arguments)
}

fn command_from_json(value: &Value) -> Option<String> {
    let object = value.as_object()?;
    for key in [
        "command",
        "cmd",
        "shell_command",
        "script",
        "input",
        "query",
    ] {
        if let Some(command) = object.get(key).and_then(Value::as_str)
            && !command.trim().is_empty()
        {
            return Some(command.to_string());
        }
    }
    None
}

fn map_rendered_lines(
    lines: &[Line<'_>],
    line_start: usize,
    visual_row: usize,
    width: u16,
    index: usize,
    row_items: &mut Vec<(usize, usize)>,
) -> usize {
    let mut visual_row = visual_row;
    for line in &lines[line_start..] {
        let height = line_visual_height(line, width);
        for row in visual_row..visual_row + height {
            row_items.push((row, index));
        }
        visual_row += height;
    }
    visual_row
}

fn line_visual_height(line: &Line<'_>, width: u16) -> usize {
    let width = usize::from(width).max(1);
    let line_width = line.width().max(1);
    line_width.div_ceil(width)
}

fn item_is_selectable(item: &TimelineItem) -> bool {
    matches!(
        item.kind,
        TimelineItemKind::Tool(_) | TimelineItemKind::Shell(_)
    )
}

fn should_separate_items(current: &TimelineItem, next: &TimelineItem) -> bool {
    match (&current.kind, &next.kind) {
        (TimelineItemKind::Tool(_), TimelineItemKind::Tool(_)) => false,
        (TimelineItemKind::Assistant { phase, .. }, TimelineItemKind::Tool(_)) => !phase
            .as_deref()
            .is_some_and(|phase| !phase.is_empty() && phase != "final_answer"),
        _ => true,
    }
}
