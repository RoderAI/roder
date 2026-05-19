mod markdown;
mod patch_preview;
mod preview;
mod render;
#[cfg(test)]
mod tests;
mod virtualization;

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    layout::Rect,
    style::Modifier,
    text::{Line, Span, Text},
};
use roder_api::events::{ThreadId, TurnId};
use roder_api::extension_state::ExtensionStateRecord;
use roder_api::interactive::{InteractiveRegion, RegionRect};
use roder_api::trace::{
    SubagentTraceDelta, SubagentTraceId, SubagentTraceStatus, SubagentTraceSummary,
};
use serde_json::Value;

use super::scroll_accel::{ScrollAccelState, ScrollDirection, ScrollSettings};
use super::stream_animation::StreamAnimator;
use super::subagent_trace::SubagentTraceRow;
use super::{Theme, short_id};
use super::{hunk_tracker::HunkTrackerRow, plan_review::PlanReviewRow};
use crate::transcript::context_menu::TranscriptContextMenu;
use crate::transcript::fold::{TranscriptFoldState, TranscriptFoldStateCodec};
use preview::tool_label;
use render::{max_scroll, visible_hit_rows};

const BOTTOM_PADDING_ROWS: usize = 3;
const MESSAGE_FOLD_LINE_LIMIT: usize = 8;
const TOOL_COLLAPSE_LIMIT: usize = 6;
const TOOL_OVERFLOW_INDEX: usize = usize::MAX;
const MIN_PAGE_SCROLL_ROWS: isize = 12;
const TIMELINE_OVERSCAN_ROWS: usize = 4;
const RUNNING_SHELL_TAIL_ROWS: usize = 12;

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
        tool_label(&self.name, &self.arguments)
    }
}

pub(super) fn fallback_entry(name: impl Into<String>) -> ToolTimelineEntry {
    ToolTimelineEntry::new(name, "")
}

#[derive(Debug, Clone, Eq, PartialEq)]
enum TimelineItemKind {
    User(String),
    Assistant(AssistantMessage),
    Reasoning(String),
    System(String),
    TurnCompleted(TurnCompletedSummary),
    Error(String),
    Shell(String),
    ShellOutput(String),
    Tool(ToolTimelineTool),
    SubagentTrace(Box<SubagentTraceRow>),
    PlanReview(Box<PlanReviewRow>),
    Hunk(Box<HunkTrackerRow>),
}

#[derive(Clone)]
struct AssistantMessage {
    text: String,
    phase: Option<String>,
    animator: StreamAnimator,
}

impl AssistantMessage {
    fn streaming(text: &str, phase: Option<String>, now: Instant) -> Self {
        let mut animator = StreamAnimator::default();
        animator.push_delta(text, now);
        Self {
            text: text.to_string(),
            phase,
            animator,
        }
    }

    fn complete(text: impl Into<String>, phase: Option<String>) -> Self {
        let text = text.into();
        let mut animator = StreamAnimator::default();
        animator.set_full_text(text.clone());
        Self {
            text,
            phase,
            animator,
        }
    }

    fn push_delta(&mut self, text: &str, now: Instant) {
        self.text.push_str(text);
        self.animator.push_delta(text, now);
    }

    fn sync_animation(&mut self) {
        self.animator.sync_to_text(&self.text);
    }
}

impl PartialEq for AssistantMessage {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text && self.phase == other.phase
    }
}

impl Eq for AssistantMessage {}

impl std::fmt::Debug for AssistantMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssistantMessage")
            .field("text", &self.text)
            .field("phase", &self.phase)
            .finish_non_exhaustive()
    }
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
    #[allow(dead_code)]
    pub scroll: u16,
    pub text_scroll: u16,
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
    subagent_trace_indices: HashMap<SubagentTraceId, usize>,
    plan_review_indices: HashMap<String, usize>,
    hunk_indices: HashMap<String, usize>,
    selected: Option<usize>,
    fold_state: TranscriptFoldState,
    focus: TimelineFocus,
    auto_follow: bool,
    scroll_offset: usize,
    last_viewport_height: u16,
    hit_rows: Vec<(u16, usize)>,
    last_area: Option<Rect>,
    context_menu: Option<TranscriptContextMenu>,
    show_all_tools: bool,
    requested_detail: Option<ToolDetail>,
    scroll_accel: ScrollAccelState,
}

impl TimelineState {
    pub fn new(scroll_settings: ScrollSettings) -> Self {
        Self {
            scroll_accel: ScrollAccelState::new(scroll_settings),
            ..Self::default()
        }
    }

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
        self.push_assistant_delta_immediate(text, phase);
    }

    pub fn push_assistant_delta_streaming(&mut self, text: &str, phase: Option<String>) {
        self.push_assistant_delta_at(text, phase, Instant::now());
    }

    #[cfg(test)]
    fn push_assistant_delta_streaming_at(
        &mut self,
        text: &str,
        phase: Option<String>,
        now: Instant,
    ) {
        self.push_assistant_delta_at(text, phase, now);
    }

    pub fn push_assistant_delta_immediate(&mut self, text: &str, phase: Option<String>) {
        if let Some(TimelineItem {
            kind: TimelineItemKind::Assistant(existing),
        }) = self.items.last_mut()
            && existing.phase == phase
        {
            existing.text.push_str(text);
            existing.sync_animation();
            self.follow_live_updates_from_composer();
            return;
        }
        self.flush_streaming_animation();
        self.push_item(TimelineItemKind::Assistant(AssistantMessage::complete(
            text.to_string(),
            phase,
        )));
    }

    fn push_assistant_delta_at(&mut self, text: &str, phase: Option<String>, now: Instant) {
        if let Some(TimelineItem {
            kind: TimelineItemKind::Assistant(existing),
        }) = self.items.last_mut()
            && existing.phase == phase
        {
            existing.push_delta(text, now);
            self.follow_live_updates_from_composer();
            return;
        }
        self.flush_streaming_animation();
        self.push_item(TimelineItemKind::Assistant(AssistantMessage::streaming(
            text, phase, now,
        )));
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

    pub fn latest_reasoning_heading(&self) -> Option<String> {
        self.items.iter().rev().find_map(|item| match &item.kind {
            TimelineItemKind::Reasoning(text) => reasoning_heading(text),
            _ => None,
        })
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
        self.flush_streaming_animation();
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
        if is_shell_like_tool(&entry.name) {
            self.collapse_previous_shell_tools();
        }
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

    #[allow(dead_code)]
    pub fn record_tool_output_delta(&mut self, tool_id: &str, output_delta: &str) {
        if output_delta.is_empty() {
            return;
        }
        let Some(index) = self.tool_indices.get(tool_id).copied() else {
            return;
        };
        if let Some(TimelineItem {
            kind: TimelineItemKind::Tool(tool),
        }) = self.items.get_mut(index)
        {
            tool.output
                .get_or_insert_with(String::new)
                .push_str(output_delta);
            self.follow_live_updates_from_composer();
        }
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
        self.flush_streaming_animation();
        self.push_item(TimelineItemKind::TurnCompleted(summary));
    }

    pub fn flush_streaming_animation(&mut self) -> bool {
        let mut changed = false;
        for item in &mut self.items {
            if let TimelineItemKind::Assistant(message) = &mut item.kind {
                changed |= message.animator.flush();
            }
        }
        changed
    }

    pub fn tick_streaming_animation(&mut self, now: Instant, width: u16) -> bool {
        let mut changed = false;
        for item in &mut self.items {
            if let TimelineItemKind::Assistant(message) = &mut item.kind {
                changed |= message.animator.tick(now, width);
            }
        }
        if changed {
            self.follow_live_updates_from_composer();
        }
        changed
    }

    pub fn has_streaming_animation(&self) -> bool {
        self.items.iter().any(|item| {
            matches!(
                &item.kind,
                TimelineItemKind::Assistant(message) if message.animator.is_animating()
            )
        })
    }

    pub fn record_subagent_trace_created(&mut self, summary: SubagentTraceSummary) {
        if let Some(index) = self.subagent_trace_indices.get(&summary.trace_id).copied() {
            if let Some(TimelineItem {
                kind: TimelineItemKind::SubagentTrace(trace),
            }) = self.items.get_mut(index)
            {
                trace.update_summary(summary);
            }
            return;
        }

        let index = self.items.len();
        self.subagent_trace_indices
            .insert(summary.trace_id.clone(), index);
        self.items.push(TimelineItem {
            kind: TimelineItemKind::SubagentTrace(Box::new(SubagentTraceRow::new(summary))),
        });
        self.follow_live_updates_from_composer();
    }

    pub fn record_subagent_trace_delta(&mut self, delta: SubagentTraceDelta) {
        if let Some(index) = self.subagent_trace_indices.get(&delta.trace_id).copied()
            && let Some(TimelineItem {
                kind: TimelineItemKind::SubagentTrace(trace),
            }) = self.items.get_mut(index)
        {
            trace.push_delta(delta);
            self.follow_live_updates_from_composer();
        }
    }

    pub fn record_subagent_trace_status(
        &mut self,
        trace_id: &str,
        status: SubagentTraceStatus,
        detail: Option<String>,
    ) {
        if let Some(index) = self.subagent_trace_indices.get(trace_id).copied()
            && let Some(TimelineItem {
                kind: TimelineItemKind::SubagentTrace(trace),
            }) = self.items.get_mut(index)
        {
            trace.update_status(status, detail);
            self.follow_live_updates_from_composer();
        }
    }

    pub fn record_subagent_trace_completed(&mut self, summary: SubagentTraceSummary) {
        self.record_subagent_trace_terminal(summary);
    }

    pub fn record_subagent_trace_failed(&mut self, summary: SubagentTraceSummary) {
        self.record_subagent_trace_terminal(summary);
    }

    pub fn record_plan_review_created(&mut self, review: roder_api::plan_review::PlanReview) {
        let index = self.items.len();
        self.plan_review_indices.insert(review.id.clone(), index);
        self.items.push(TimelineItem {
            kind: TimelineItemKind::PlanReview(Box::new(PlanReviewRow::new(review))),
        });
        self.follow_live_updates_from_composer();
    }

    pub fn record_plan_review_status(
        &mut self,
        review_id: &str,
        status: roder_api::plan_review::PlanReviewStatus,
    ) {
        if let Some(index) = self.plan_review_indices.get(review_id).copied()
            && let Some(TimelineItem {
                kind: TimelineItemKind::PlanReview(row),
            }) = self.items.get_mut(index)
        {
            row.update_status(status);
            self.follow_live_updates_from_composer();
        }
    }

    pub fn record_plan_review_comment(&mut self, comment: roder_api::plan_review::PlanComment) {
        if let Some(index) = self.plan_review_indices.get(&comment.review_id).copied()
            && let Some(TimelineItem {
                kind: TimelineItemKind::PlanReview(row),
            }) = self.items.get_mut(index)
        {
            row.push_comment(comment);
            self.follow_live_updates_from_composer();
        }
    }

    pub fn record_plan_review_rewrite(&mut self, rewrite: roder_api::plan_review::PlanRewrite) {
        if let Some(index) = self.plan_review_indices.get(&rewrite.review_id).copied()
            && let Some(TimelineItem {
                kind: TimelineItemKind::PlanReview(row),
            }) = self.items.get_mut(index)
        {
            row.push_rewrite(rewrite);
            self.follow_live_updates_from_composer();
        }
    }

    pub fn record_hunk(&mut self, hunk: roder_api::plan_review::HunkRecord) {
        if hunk.plan_review_id.is_some() {
            return;
        }
        let index = self.items.len();
        self.hunk_indices.insert(hunk.id.clone(), index);
        self.items.push(TimelineItem {
            kind: TimelineItemKind::Hunk(Box::new(HunkTrackerRow::new(hunk))),
        });
        self.follow_live_updates_from_composer();
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
                self.scroll_by_wheel(ScrollDirection::Down);
                return true;
            }
            MouseEventKind::ScrollUp => {
                self.focus = TimelineFocus::Timeline;
                self.scroll_by_wheel(ScrollDirection::Up);
                return true;
            }
            MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left) => {}
            MouseEventKind::Down(MouseButton::Right) | MouseEventKind::Up(MouseButton::Right) => {}
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
        if matches!(
            event.kind,
            MouseEventKind::Down(MouseButton::Right) | MouseEventKind::Up(MouseButton::Right)
        ) {
            return self.open_context_menu(index, event.column, event.row);
        }
        if self
            .items
            .get(index)
            .is_some_and(item_is_foldable_assistant)
        {
            self.toggle_expansion(index);
            return true;
        }
        if !self.items.get(index).is_some_and(item_is_mouse_selectable) {
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

    #[allow(dead_code)]
    pub fn fold_state_record(
        &self,
        thread_id: impl Into<ThreadId>,
    ) -> anyhow::Result<ExtensionStateRecord> {
        TranscriptFoldStateCodec::thread(thread_id).encode(&self.fold_state)
    }

    #[allow(dead_code)]
    pub fn restore_fold_state_record(
        &mut self,
        record: &ExtensionStateRecord,
        thread_id: impl Into<ThreadId>,
    ) -> anyhow::Result<()> {
        self.fold_state = TranscriptFoldStateCodec::thread(thread_id).decode(record)?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn interactive_regions(
        &self,
        area: Rect,
        thread_id: &str,
        turn_id: &str,
    ) -> Vec<InteractiveRegion> {
        use std::collections::BTreeMap;

        let mut rows_by_index = BTreeMap::<usize, (u16, u16)>::new();
        for (row, index) in &self.hit_rows {
            rows_by_index
                .entry(*index)
                .and_modify(|range| range.1 = range.1.max(*row))
                .or_insert((*row, *row));
        }

        let mut regions = rows_by_index
            .into_iter()
            .filter_map(|(index, (start, end))| {
                let rect = RegionRect {
                    x: area.x,
                    y: start,
                    width: area.width,
                    height: end.saturating_sub(start).saturating_add(1),
                };
                let item = self.items.get(index)?;
                let mut regions = match item {
                    TimelineItem {
                        kind: TimelineItemKind::Tool(tool),
                    } => vec![crate::transcript::regions::tool_call_region(
                        format!("tool-call-{}", tool.tool_id),
                        rect,
                        10,
                        tool.tool_id.clone(),
                        self.fold_state.is_expanded(&tool.tool_id),
                    )],
                    _ => vec![crate::transcript::regions::transcript_message_region(
                        format!("transcript-message-{index}"),
                        rect,
                        0,
                        ThreadId::from(thread_id),
                        TurnId::from(turn_id),
                        index,
                    )],
                };
                if let Some(text) = item.transcript_text() {
                    for (link_index, link) in
                        crate::transcript::links::linkify_transcript_text(text)
                            .into_iter()
                            .enumerate()
                    {
                        match link {
                            crate::transcript::links::TranscriptLink::Url { text } => {
                                regions.push(crate::transcript::regions::url_region(
                                    format!("transcript-url-{index}-{link_index}"),
                                    rect,
                                    20,
                                    text,
                                ));
                            }
                            crate::transcript::links::TranscriptLink::FileReference {
                                path,
                                line,
                            } => regions.push(crate::transcript::regions::file_reference_region(
                                format!("transcript-file-{index}-{link_index}"),
                                rect,
                                20,
                                path,
                                line,
                            )),
                        }
                    }
                }
                Some(regions)
            })
            .flatten()
            .collect::<Vec<_>>();

        if let Some(context_menu) = &self.context_menu {
            regions.extend(context_menu.regions());
        }

        regions
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
        self.last_area = Some(area);
        let max_scroll = max_scroll(visual_height, area.height);
        let scroll = self.scroll_for(area.height, &row_items, max_scroll);
        self.hit_rows = visible_hit_rows(area, scroll, area.height, &row_items);
        self.scroll_offset = usize::from(scroll);
        let window = render_window_lines(
            lines,
            usize::from(scroll),
            usize::from(area.height),
            TIMELINE_OVERSCAN_ROWS,
            area.width,
        );
        TimelineRender {
            text: Text::from(window.lines),
            scroll,
            text_scroll: window.text_scroll,
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
        self.scroll_accel.reset();
        let current = self.scroll_offset as isize;
        self.scroll_offset = current.saturating_add(amount).max(0) as usize;
        self.selected = None;
        self.auto_follow = false;
    }

    fn scroll_by_wheel(&mut self, direction: ScrollDirection) {
        let rows = self.scroll_accel.tick(direction, std::time::Instant::now());
        let signed_rows = match direction {
            ScrollDirection::Down => rows,
            ScrollDirection::Up => -rows,
        };
        let current = self.scroll_offset as isize;
        self.scroll_offset = current.saturating_add(signed_rows).max(0) as usize;
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
        if let Some(key) = self.fold_key_for_index(index) {
            self.fold_state.toggle(key);
        }
    }

    fn collapse_previous_shell_tools(&mut self) {
        for item in &self.items {
            let TimelineItemKind::Tool(tool) = &item.kind else {
                continue;
            };
            if is_shell_like_tool(&tool.entry.name) {
                self.fold_state.set_expanded(tool.tool_id.clone(), false);
            }
        }
    }

    fn record_subagent_trace_terminal(&mut self, summary: SubagentTraceSummary) {
        if let Some(index) = self.subagent_trace_indices.get(&summary.trace_id).copied()
            && let Some(TimelineItem {
                kind: TimelineItemKind::SubagentTrace(trace),
            }) = self.items.get_mut(index)
        {
            trace.update_summary(summary);
            self.follow_live_updates_from_composer();
            return;
        }
        self.record_subagent_trace_created(summary);
    }

    fn fold_key_for_index(&self, index: usize) -> Option<String> {
        match self.items.get(index)? {
            TimelineItem {
                kind: TimelineItemKind::Tool(tool),
            } => Some(tool.tool_id.clone()),
            TimelineItem {
                kind: TimelineItemKind::SubagentTrace(trace),
            } => Some(trace.trace_id().to_string()),
            TimelineItem {
                kind: TimelineItemKind::PlanReview(review),
            } => Some(review.review_id().to_string()),
            TimelineItem {
                kind: TimelineItemKind::Hunk(hunk),
            } => Some(hunk.hunk_id().to_string()),
            TimelineItem {
                kind:
                    TimelineItemKind::User(_)
                    | TimelineItemKind::Assistant(_)
                    | TimelineItemKind::System(_)
                    | TimelineItemKind::Error(_)
                    | TimelineItemKind::Shell(_)
                    | TimelineItemKind::ShellOutput(_),
            } => Some(format!("message-{index}")),
            _ => None,
        }
    }

    fn item_can_expand(&self, index: usize) -> bool {
        if index == TOOL_OVERFLOW_INDEX {
            return !self.show_all_tools && self.hidden_tool_count() > 0;
        }
        match self.items.get(index) {
            Some(TimelineItem {
                kind:
                    TimelineItemKind::Tool(ToolTimelineTool {
                        output: Some(output),
                        ..
                    }),
            }) => !output.trim().is_empty(),
            Some(TimelineItem {
                kind: TimelineItemKind::SubagentTrace(trace),
            }) => trace.has_items(),
            Some(TimelineItem {
                kind: TimelineItemKind::PlanReview(review),
            }) => review.can_expand(),
            Some(TimelineItem {
                kind: TimelineItemKind::Hunk(hunk),
            }) => hunk.can_expand(),
            Some(item) => item_is_foldable_message(item),
            None => false,
        }
    }

    fn open_context_menu(&mut self, index: usize, column: u16, row: u16) -> bool {
        let Some(item) = self.items.get(index) else {
            return false;
        };
        if item.transcript_text().is_none() {
            return false;
        }
        let viewport = self
            .last_area
            .unwrap_or_else(|| Rect::new(0, 0, u16::MAX, self.last_viewport_height.max(1)));
        self.context_menu = Some(TranscriptContextMenu::at_message(
            format!("transcript-message-{index}"),
            (column, row),
            viewport,
            self.item_can_expand(index),
            true,
        ));
        true
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
        let latest_reasoning_index = self.latest_reasoning_index();
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
            // Honor `display: none` for `.timeline-thinking`. This is the
            // headline RFC demo — themes like minimal.css hide the model's
            // chain-of-thought.
            if !item_is_visible(item, index, latest_reasoning_index, theme.hide_thinking) {
                continue;
            }
            let line_start = lines.len();
            let selected = self.focus == TimelineFocus::Timeline
                && self.selected == Some(index)
                && item_is_selectable(item);
            let expanded = self
                .fold_key_for_index(index)
                .is_some_and(|key| self.fold_state.is_expanded(&key));
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
            if self.should_separate_visible_item(
                index,
                &visible_tools,
                latest_reasoning_index,
                theme.hide_thinking,
            ) {
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

    fn latest_reasoning_index(&self) -> Option<usize> {
        self.items.iter().rposition(|item| {
            matches!(
                item.kind,
                TimelineItemKind::Reasoning(ref text) if !reasoning_visible_body(text).trim().is_empty()
            )
        })
    }

    fn should_separate_visible_item(
        &self,
        index: usize,
        visible_tools: &[usize],
        latest_reasoning_index: Option<usize>,
        hide_thinking: bool,
    ) -> bool {
        let Some(next_index) =
            self.next_visible_index(index, visible_tools, latest_reasoning_index, hide_thinking)
        else {
            return false;
        };
        should_separate_items(&self.items[index], &self.items[next_index])
    }

    fn next_visible_index(
        &self,
        index: usize,
        visible_tools: &[usize],
        latest_reasoning_index: Option<usize>,
        hide_thinking: bool,
    ) -> Option<usize> {
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
                if !item_is_visible(item, candidate, latest_reasoning_index, hide_thinking) {
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

pub(super) fn is_shell_like_tool(name: &str) -> bool {
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

struct RenderWindowLines {
    lines: Vec<Line<'static>>,
    text_scroll: u16,
}

fn render_window_lines(
    lines: Vec<Line<'static>>,
    scroll: usize,
    height: usize,
    overscan_rows: usize,
    width: u16,
) -> RenderWindowLines {
    if height == 0 || lines.is_empty() {
        return RenderWindowLines {
            lines: Vec::new(),
            text_scroll: 0,
        };
    }

    let render_top = scroll.saturating_sub(overscan_rows);
    let render_bottom = scroll.saturating_add(height).saturating_add(overscan_rows);
    let mut visual_row = 0usize;
    let mut first_line_start = 0usize;
    let mut window = Vec::new();
    for line in lines {
        let line_start = visual_row;
        let line_height = line_visual_height(&line, width);
        let line_end = line_start.saturating_add(line_height);
        visual_row = line_end;
        if line_end <= render_top {
            first_line_start = line_end;
            continue;
        }
        if line_start >= render_bottom {
            break;
        }
        window.push(line);
    }

    RenderWindowLines {
        lines: window,
        text_scroll: scroll.saturating_sub(first_line_start) as u16,
    }
}

fn line_visual_height(line: &Line<'_>, width: u16) -> usize {
    let width = usize::from(width).max(1);
    let line_width = line.width().max(1);
    line_width.div_ceil(width)
}

fn item_is_selectable(item: &TimelineItem) -> bool {
    matches!(
        item.kind,
        TimelineItemKind::Tool(_)
            | TimelineItemKind::Shell(_)
            | TimelineItemKind::SubagentTrace(_)
            | TimelineItemKind::PlanReview(_)
            | TimelineItemKind::Hunk(_)
    ) || item_is_foldable_message(item)
}

fn item_is_mouse_selectable(item: &TimelineItem) -> bool {
    if matches!(item.kind, TimelineItemKind::Assistant(_)) {
        return false;
    }
    item_is_selectable(item)
}

fn item_is_foldable_assistant(item: &TimelineItem) -> bool {
    matches!(item.kind, TimelineItemKind::Assistant(_)) && item_is_foldable_message(item)
}

fn item_is_foldable_message(item: &TimelineItem) -> bool {
    item.transcript_text()
        .is_some_and(|text| text.lines().count() > MESSAGE_FOLD_LINE_LIMIT)
}

fn item_is_visible(
    item: &TimelineItem,
    index: usize,
    latest_reasoning_index: Option<usize>,
    hide_thinking: bool,
) -> bool {
    match item.kind {
        TimelineItemKind::Reasoning(_) => !hide_thinking && Some(index) == latest_reasoning_index,
        _ => true,
    }
}

fn reasoning_heading(text: &str) -> Option<String> {
    text.lines()
        .find(|line| !line.trim().is_empty())
        .and_then(parse_bold_heading_line)
}

pub(super) fn reasoning_visible_body(text: &str) -> String {
    let lines = text.lines().collect::<Vec<_>>();
    let Some(heading_index) = lines.iter().position(|line| !line.trim().is_empty()) else {
        return String::new();
    };
    if parse_bold_heading_line(lines[heading_index]).is_none() {
        return text.to_string();
    }
    let body_start = lines
        .iter()
        .enumerate()
        .skip(heading_index + 1)
        .find_map(|(index, line)| (!line.trim().is_empty()).then_some(index))
        .unwrap_or(lines.len());
    lines[body_start..].join("\n")
}

#[allow(dead_code)]
impl TimelineItem {
    fn transcript_text(&self) -> Option<&str> {
        match &self.kind {
            TimelineItemKind::User(text)
            | TimelineItemKind::Reasoning(text)
            | TimelineItemKind::System(text)
            | TimelineItemKind::Error(text)
            | TimelineItemKind::Shell(text)
            | TimelineItemKind::ShellOutput(text) => Some(text),
            TimelineItemKind::Assistant(message) => Some(&message.text),
            TimelineItemKind::Tool(_)
            | TimelineItemKind::SubagentTrace(_)
            | TimelineItemKind::PlanReview(_)
            | TimelineItemKind::Hunk(_)
            | TimelineItemKind::TurnCompleted(_) => None,
        }
    }
}

fn parse_bold_heading_line(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let inner = trimmed.strip_prefix("**")?.strip_suffix("**")?.trim();
    (!inner.is_empty()).then(|| inner.to_string())
}

fn should_separate_items(current: &TimelineItem, next: &TimelineItem) -> bool {
    match (&current.kind, &next.kind) {
        (TimelineItemKind::Tool(_), TimelineItemKind::Tool(_)) => false,
        (TimelineItemKind::SubagentTrace(_), TimelineItemKind::SubagentTrace(_)) => false,
        (TimelineItemKind::Assistant(message), TimelineItemKind::Tool(_)) => !message
            .phase
            .as_deref()
            .is_some_and(|phase| !phase.is_empty() && phase != "final_answer"),
        _ => true,
    }
}
