mod markdown;
mod patch_preview;
mod preview;
mod render;
#[cfg(test)]
mod tests;
mod virtualization;

use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
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

use super::plan_review::PlanReviewRow;
use roder_tui_util::scroll_accel::{ScrollAccelState, ScrollDirection, ScrollSettings};
use super::stream_animation::StreamAnimator;
use super::subagent_trace::SubagentTraceRow;
use super::{Theme, short_id};
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

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub(super) struct TimelineSettings {
    pub message_folding: bool,
}

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
    Reasoning(AnimatedMessage),
    System(String),
    TurnCompleted(TurnCompletedSummary),
    Error(String),
    Shell(String),
    ShellOutput(String),
    Tool(ToolTimelineTool),
    SubagentTrace(Box<SubagentTraceRow>),
    PlanReview(Box<PlanReviewRow>),
}

#[derive(Clone)]
struct AnimatedMessage {
    text: String,
    animator: StreamAnimator,
}

impl AnimatedMessage {
    fn streaming(text: &str, now: Instant) -> Self {
        let mut animator = StreamAnimator::default();
        animator.push_delta(text, now);
        Self {
            text: text.to_string(),
            animator,
        }
    }

    fn complete(text: impl Into<String>) -> Self {
        let text = text.into();
        let mut animator = StreamAnimator::default();
        animator.set_full_text(text.clone());
        Self { text, animator }
    }

    fn push_delta(&mut self, text: &str, now: Instant) {
        self.text.push_str(text);
        self.animator.push_delta(text, now);
    }

    fn sync_animation(&mut self) {
        self.animator.sync_to_text(&self.text);
    }
}

impl PartialEq for AnimatedMessage {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
    }
}

impl Eq for AnimatedMessage {}

impl std::fmt::Debug for AnimatedMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnimatedMessage")
            .field("text", &self.text)
            .finish_non_exhaustive()
    }
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
    pub thread_tokens: u64,
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
    started_at: time::OffsetDateTime,
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

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct TimelineRenderCacheKey {
    width: u16,
    theme: Theme,
    selected: Option<usize>,
    show_all_tools: bool,
    message_folding: bool,
    fold_state_hash: u64,
}

#[derive(Debug, Clone)]
struct TimelineRenderCache {
    key: TimelineRenderCacheKey,
    lines: Vec<Line<'static>>,
    row_items: Vec<(usize, usize)>,
    visual_height: usize,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(super) struct ToolDetail {
    pub tool_id: Option<String>,
    pub title: String,
    pub command: Option<String>,
    pub arguments: String,
    pub output: Option<String>,
    pub failed: bool,
    pub running: bool,
}

#[derive(Debug, Default)]
pub(super) struct TimelineState {
    items: Vec<TimelineItem>,
    tool_indices: HashMap<String, usize>,
    subagent_trace_indices: HashMap<SubagentTraceId, usize>,
    plan_review_indices: HashMap<String, usize>,
    render_cache: Option<TimelineRenderCache>,
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
    settings: TimelineSettings,
    turn_started_at: Option<time::OffsetDateTime>,
}

impl TimelineState {
    pub fn new(scroll_settings: ScrollSettings, settings: TimelineSettings) -> Self {
        Self {
            scroll_accel: ScrollAccelState::new(scroll_settings),
            settings,
            ..Self::default()
        }
    }

    pub fn set_settings(&mut self, settings: TimelineSettings) {
        self.settings = settings;
        self.invalidate_render_cache();
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
        self.invalidate_render_cache();
    }

    pub fn focus_composer(&mut self) {
        self.focus = TimelineFocus::Composer;
        self.selected = None;
        self.auto_follow = true;
        self.invalidate_render_cache();
    }

    pub fn follow_latest(&mut self) {
        self.auto_follow = true;
        self.selected = None;
        self.invalidate_render_cache();
    }

    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    pub fn prepend_system(&mut self, text: impl Into<String>) {
        self.prepend_item(TimelineItemKind::System(text.into()));
    }

    pub fn preserve_scroll_after_prepend(&mut self, added_visual_rows: usize) {
        self.scroll_offset = self.scroll_offset.saturating_add(added_visual_rows);
        self.auto_follow = false;
    }

    pub fn last_viewport_height(&self) -> u16 {
        self.last_viewport_height
    }

    pub fn visual_height(&self) -> usize {
        self.render_cache
            .as_ref()
            .map(|cache| cache.visual_height)
            .unwrap_or_default()
    }

    pub fn push_user(&mut self, text: impl Into<String>) {
        self.auto_follow = true;
        self.selected = None;
        self.push_item(TimelineItemKind::User(text.into()));
    }

    pub fn prepend_user(&mut self, text: impl Into<String>) {
        self.prepend_item(TimelineItemKind::User(text.into()));
    }

    pub fn prepend_assistant(&mut self, text: impl Into<String>, phase: Option<String>) {
        self.prepend_item(TimelineItemKind::Assistant(AssistantMessage::complete(
            text.into(),
            phase,
        )));
    }

    pub fn prepend_reasoning(&mut self, text: impl Into<String>) {
        self.prepend_item(TimelineItemKind::Reasoning(AnimatedMessage::complete(
            text.into(),
        )));
    }

    pub fn prepend_error(&mut self, text: impl Into<String>) {
        self.prepend_item(TimelineItemKind::Error(text.into()));
    }

    #[cfg(test)]
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
            self.invalidate_render_cache();
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
            self.invalidate_render_cache();
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
            existing.text.push_str(text);
            existing.sync_animation();
            self.invalidate_render_cache();
            self.follow_live_updates_from_composer();
            return;
        }
        self.flush_streaming_animation();
        self.push_item(TimelineItemKind::Reasoning(AnimatedMessage::complete(text)));
    }

    pub fn push_reasoning_delta_streaming(&mut self, text: &str) {
        self.push_reasoning_delta_streaming_at(text, Instant::now());
    }

    #[cfg(test)]
    fn push_reasoning_delta_streaming_at_for_test(&mut self, text: &str, now: Instant) {
        self.push_reasoning_delta_streaming_at(text, now);
    }

    fn push_reasoning_delta_streaming_at(&mut self, text: &str, now: Instant) {
        if let Some(TimelineItem {
            kind: TimelineItemKind::Reasoning(existing),
        }) = self.items.last_mut()
        {
            existing.push_delta(text, now);
            self.invalidate_render_cache();
            self.follow_live_updates_from_composer();
            return;
        }
        self.flush_streaming_animation();
        self.push_item(TimelineItemKind::Reasoning(AnimatedMessage::streaming(
            text, now,
        )));
    }

    pub fn latest_reasoning_heading(&self) -> Option<String> {
        self.items.iter().rev().find_map(|item| match &item.kind {
            TimelineItemKind::Reasoning(message) => reasoning_heading(&message.text),
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
                self.mutate_item(index);
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
                started_at: time::OffsetDateTime::now_local()
                    .unwrap_or_else(|_| time::OffsetDateTime::now_utc()),
            }),
        });
        self.invalidate_render_cache();
        self.tool_indices.insert(tool_id, index);
        self.follow_live_updates_from_composer();
    }

    pub fn remove_tool(&mut self, tool_id: &str) -> bool {
        let Some(index) = self.tool_indices.remove(tool_id) else {
            return false;
        };
        self.items.remove(index);
        self.invalidate_render_cache();
        shift_indices_after_removal(&mut self.tool_indices, index);
        shift_indices_after_removal(&mut self.subagent_trace_indices, index);
        shift_indices_after_removal(&mut self.plan_review_indices, index);
        self.selected = match self.selected {
            Some(selected) if selected == index => None,
            Some(selected) if selected > index => Some(selected - 1),
            selected => selected,
        };
        self.hit_rows.clear();
        self.follow_live_updates_from_composer();
        true
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
            self.mutate_item(index);
            self.follow_live_updates_from_composer();
        }
    }

    pub fn record_tool_session_update(
        &mut self,
        tool_id: &str,
        failed: bool,
        output_delta: Option<String>,
        still_running: bool,
    ) {
        let Some(index) = self.tool_indices.get(tool_id).copied() else {
            return;
        };
        if let Some(TimelineItem {
            kind: TimelineItemKind::Tool(tool),
        }) = self.items.get_mut(index)
        {
            tool.status = if failed {
                ToolTimelineStatus::Failed
            } else if still_running {
                ToolTimelineStatus::Running
            } else {
                ToolTimelineStatus::Completed
            };
            if let Some(delta) = output_delta
                .as_deref()
                .filter(|text| !text.trim().is_empty())
            {
                append_tool_output(&mut tool.output, delta);
            }
            self.mutate_item(index);
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
            self.mutate_item(index);
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
            self.mutate_item(index);
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
            match &mut item.kind {
                TimelineItemKind::Assistant(message) => changed |= message.animator.flush(),
                TimelineItemKind::Reasoning(message) => changed |= message.animator.flush(),
                _ => {}
            }
        }
        if changed {
            self.invalidate_render_cache();
        }
        changed
    }

    pub fn tick_streaming_animation(&mut self, now: Instant, width: u16) -> bool {
        let mut changed = false;
        for item in &mut self.items {
            match &mut item.kind {
                TimelineItemKind::Assistant(message) => {
                    changed |= message.animator.tick(now, width);
                }
                TimelineItemKind::Reasoning(message) => {
                    changed |= message.animator.tick(now, width);
                }
                _ => {}
            }
        }
        if changed {
            self.invalidate_render_cache();
            self.follow_live_updates_from_composer();
        }
        changed
    }

    pub fn has_streaming_animation(&self) -> bool {
        self.items.iter().any(|item| {
            matches!(
                &item.kind,
                TimelineItemKind::Assistant(message) if message.animator.is_animating()
            ) || matches!(
                &item.kind,
                TimelineItemKind::Reasoning(message) if message.animator.is_animating()
            )
        })
    }

    fn has_dynamic_render(&self) -> bool {
        self.has_streaming_animation()
            || self.items.iter().any(|item| {
                matches!(
                    &item.kind,
                    TimelineItemKind::Tool(tool) if tool.status == ToolTimelineStatus::Running
                )
            })
    }

    fn invalidate_render_cache(&mut self) {
        self.render_cache = None;
    }

    fn mutate_item(&mut self, index: usize) {
        if index < self.items.len() {
            self.invalidate_render_cache();
        }
    }

    pub fn record_subagent_trace_created(&mut self, summary: SubagentTraceSummary) {
        if let Some(index) = self.subagent_trace_indices.get(&summary.trace_id).copied() {
            if let Some(TimelineItem {
                kind: TimelineItemKind::SubagentTrace(trace),
            }) = self.items.get_mut(index)
            {
                trace.update_summary(summary);
                self.mutate_item(index);
            }
            return;
        }

        let index = self.items.len();
        self.subagent_trace_indices
            .insert(summary.trace_id.clone(), index);
        self.items.push(TimelineItem {
            kind: TimelineItemKind::SubagentTrace(Box::new(SubagentTraceRow::new(summary))),
        });
        self.invalidate_render_cache();
        self.follow_live_updates_from_composer();
    }

    fn prepend_item(&mut self, kind: TimelineItemKind) {
        self.items.insert(0, TimelineItem { kind });
        self.rebuild_item_indices();
        self.invalidate_render_cache();
    }

    fn rebuild_item_indices(&mut self) {
        self.tool_indices.clear();
        self.subagent_trace_indices.clear();
        self.plan_review_indices.clear();
        for (index, item) in self.items.iter().enumerate() {
            match &item.kind {
                TimelineItemKind::Tool(tool) => {
                    self.tool_indices.insert(tool.tool_id.clone(), index);
                }
                TimelineItemKind::SubagentTrace(trace) => {
                    self.subagent_trace_indices
                        .insert(trace.trace_id().to_string(), index);
                }
                TimelineItemKind::PlanReview(review) => {
                    self.plan_review_indices
                        .insert(review.review_id().to_string(), index);
                }
                _ => {}
            }
        }
    }

    pub fn record_subagent_trace_delta(&mut self, delta: SubagentTraceDelta) {
        if let Some(index) = self.subagent_trace_indices.get(&delta.trace_id).copied()
            && let Some(TimelineItem {
                kind: TimelineItemKind::SubagentTrace(trace),
            }) = self.items.get_mut(index)
        {
            trace.push_delta(delta);
            self.mutate_item(index);
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
            self.mutate_item(index);
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
        self.invalidate_render_cache();
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
            self.mutate_item(index);
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
            self.mutate_item(index);
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
            self.mutate_item(index);
            self.follow_live_updates_from_composer();
        }
    }

    pub fn record_hunk(&mut self, _hunk: roder_api::plan_review::HunkRecord) {
        // Hunk records are rollback metadata; rendering them produced noisy restore rows.
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
                self.invalidate_render_cache();
                true
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.select_previous();
                self.invalidate_render_cache();
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
                self.invalidate_render_cache();
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
            MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Down(MouseButton::Right) => {}
            // A terminal click commonly arrives as Down followed by Up. Only the
            // Down edge should activate rows; otherwise a click on an already
            // expanded tool collapses on Down and immediately re-expands on Up.
            MouseEventKind::Up(MouseButton::Left) | MouseEventKind::Up(MouseButton::Right) => {
                return self.hit_rows.iter().any(|(row, _)| *row == event.row);
            }
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
        if matches!(event.kind, MouseEventKind::Down(MouseButton::Right)) {
            return self.open_context_menu(index, event.column, event.row);
        }
        if self
            .items
            .get(index)
            .is_some_and(|item| self.item_is_foldable_assistant(item))
        {
            self.toggle_expansion(index);
            return true;
        }
        if !self
            .items
            .get(index)
            .is_some_and(|item| self.item_is_mouse_selectable(item))
        {
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

    pub fn detail_for_tool_id(&self, tool_id: &str) -> Option<ToolDetail> {
        let index = self.tool_indices.get(tool_id).copied()?;
        self.detail_for_index(index)
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
        self.prepare_render_cache(theme, area.width, animation_frame);
        self.last_viewport_height = area.height;
        self.last_area = Some(area);
        let cached_render = self
            .render_cache
            .as_ref()
            .expect("render cache is populated before rendering");
        let max_scroll = max_scroll(cached_render.visual_height, area.height);
        if !self.auto_follow && self.scroll_offset >= max_scroll {
            self.auto_follow = true;
        }
        let scroll = scroll_for_state(
            self.focus,
            self.selected,
            self.auto_follow,
            self.scroll_offset,
            area.height,
            &cached_render.row_items,
            max_scroll,
        );
        self.hit_rows = visible_hit_rows(area, scroll, area.height, &cached_render.row_items);
        self.scroll_offset = usize::from(scroll);
        let window = render_window_lines(
            &cached_render.lines,
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

    fn prepare_render_cache(&mut self, theme: Theme, width: u16, animation_frame: u64) {
        if self.has_dynamic_render() {
            let (lines, row_items, visual_height) = self.build_lines(theme, width, animation_frame);
            self.render_cache = Some(TimelineRenderCache {
                key: self.render_cache_key(theme, width),
                lines,
                row_items,
                visual_height,
            });
            return;
        }

        let key = self.render_cache_key(theme, width);
        if let Some(cache) = &self.render_cache
            && cache.key == key
        {
            return;
        }

        let (lines, row_items, visual_height) = self.build_lines(theme, width, animation_frame);
        self.render_cache = Some(TimelineRenderCache {
            key,
            lines,
            row_items,
            visual_height,
        });
    }

    fn render_cache_key(&self, theme: Theme, width: u16) -> TimelineRenderCacheKey {
        TimelineRenderCacheKey {
            width,
            theme,
            selected: (self.focus == TimelineFocus::Timeline)
                .then_some(self.selected)
                .flatten(),
            show_all_tools: self.show_all_tools,
            message_folding: self.settings.message_folding,
            fold_state_hash: fold_state_hash(&self.fold_state),
        }
    }

    fn push_item(&mut self, kind: TimelineItemKind) {
        if let TimelineItemKind::User(_) = &kind {
            self.turn_started_at = Some(
                time::OffsetDateTime::now_local()
                    .unwrap_or_else(|_| time::OffsetDateTime::now_utc()),
            );
        }
        self.items.push(TimelineItem { kind });
        self.invalidate_render_cache();
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
            .filter_map(|(index, item)| self.item_is_selectable(item).then_some(index))
            .collect()
    }

    fn item_is_selectable(&self, item: &TimelineItem) -> bool {
        item_is_non_message_selectable(item)
            || (self.settings.message_folding && item_is_foldable_message(item))
    }

    fn item_is_mouse_selectable(&self, item: &TimelineItem) -> bool {
        if matches!(item.kind, TimelineItemKind::Assistant(_)) {
            return false;
        }
        self.item_is_selectable(item)
    }

    fn item_is_foldable_assistant(&self, item: &TimelineItem) -> bool {
        self.settings.message_folding
            && matches!(item.kind, TimelineItemKind::Assistant(_))
            && item_is_foldable_message(item)
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
            self.invalidate_render_cache();
        }
    }

    fn collapse_previous_shell_tools(&mut self) {
        let mut changed = false;
        for item in &self.items {
            let TimelineItemKind::Tool(tool) = &item.kind else {
                continue;
            };
            if is_shell_like_tool(&tool.entry.name) {
                self.fold_state.set_expanded(tool.tool_id.clone(), false);
                changed = true;
            }
        }
        if changed {
            self.invalidate_render_cache();
        }
    }

    fn record_subagent_trace_terminal(&mut self, summary: SubagentTraceSummary) {
        if let Some(index) = self.subagent_trace_indices.get(&summary.trace_id).copied()
            && let Some(TimelineItem {
                kind: TimelineItemKind::SubagentTrace(trace),
            }) = self.items.get_mut(index)
        {
            trace.update_summary(summary);
            self.mutate_item(index);
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
            Some(item) => self.settings.message_folding && item_is_foldable_message(item),
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
                tool_id: None,
                title: "Shell".to_string(),
                command: Some(command.clone()),
                arguments: String::new(),
                output: self.shell_output_after(index),
                failed: false,
                running: false,
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
        scroll_for_state(
            self.focus,
            self.selected,
            self.auto_follow,
            self.scroll_offset,
            height,
            row_items,
            max_scroll,
        )
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
        let tool_visibility = self.tool_visibility();
        let visible_tools = &tool_visibility.visible;
        let mut overflow_insertions = tool_visibility.overflow_insertions.iter().peekable();
        let latest_reasoning_index = self.latest_reasoning_index();
        let mut last_timestamp = self.turn_started_at;
        for (index, item) in self.items.iter().enumerate() {
            while let Some(overflow) = overflow_insertions.next_if(|overflow| overflow.0 == index) {
                let line_start = lines.len();
                render::push_tool_overflow_line(
                    overflow.1,
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
                && self.item_is_selectable(item);
            let expanded = self
                .fold_key_for_index(index)
                .is_some_and(|key| self.fold_state.is_expanded(&key));
            item.render(
                selected,
                expanded,
                theme,
                width,
                animation_frame,
                self.settings.message_folding,
                last_timestamp,
                &mut lines,
            );
            if let TimelineItemKind::Tool(tool) = &item.kind {
                last_timestamp = Some(tool.started_at);
            }
            visual_row =
                map_rendered_lines(&lines, line_start, visual_row, width, index, &mut row_items);
            if self.should_separate_visible_item(
                index,
                visible_tools,
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

    fn tool_visibility(&self) -> ToolVisibility {
        if self.show_all_tools {
            let visible = self
                .items
                .iter()
                .enumerate()
                .filter_map(|(index, item)| {
                    matches!(item.kind, TimelineItemKind::Tool(_)).then_some(index)
                })
                .collect();
            return ToolVisibility {
                visible,
                overflow_insertions: Vec::new(),
            };
        }

        let mut visibility = ToolVisibility::default();
        let mut group = Vec::new();
        for (index, item) in self.items.iter().enumerate() {
            if matches!(item.kind, TimelineItemKind::Assistant(_)) {
                push_collapsed_tool_group(&mut visibility, &group);
                group.clear();
            }
            if matches!(item.kind, TimelineItemKind::Tool(_)) {
                group.push(index);
            }
        }
        push_collapsed_tool_group(&mut visibility, &group);
        visibility
    }

    fn visible_tool_indices(&self) -> Vec<usize> {
        self.tool_visibility().visible
    }

    fn hidden_tool_count(&self) -> usize {
        self.tool_visibility()
            .overflow_insertions
            .iter()
            .map(|(_, hidden_count)| hidden_count)
            .sum()
    }

    fn latest_reasoning_index(&self) -> Option<usize> {
        self.items.iter().rposition(|item| {
            matches!(
                &item.kind,
                TimelineItemKind::Reasoning(message) if !reasoning_visible_body(&message.text).trim().is_empty()
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

fn scroll_for_state(
    focus: TimelineFocus,
    selected: Option<usize>,
    auto_follow: bool,
    scroll_offset: usize,
    height: u16,
    row_items: &[(usize, usize)],
    max_scroll: usize,
) -> u16 {
    if row_items.is_empty() || height == 0 {
        return 0;
    }
    if auto_follow {
        return max_scroll as u16;
    }
    if focus == TimelineFocus::Timeline
        && let Some(selected) = selected
        && let Some((row, _)) = row_items.iter().find(|(_, index)| *index == selected)
    {
        let half = usize::from(height) / 2;
        return row.saturating_sub(half).min(max_scroll) as u16;
    }
    scroll_offset.min(max_scroll) as u16
}

#[derive(Debug, Default, Clone, Eq, PartialEq)]
struct ToolVisibility {
    visible: Vec<usize>,
    overflow_insertions: Vec<(usize, usize)>,
}

fn push_collapsed_tool_group(visibility: &mut ToolVisibility, group: &[usize]) {
    if group.len() <= TOOL_COLLAPSE_LIMIT {
        visibility.visible.extend_from_slice(group);
        return;
    }

    let visible_start = group.len() - TOOL_COLLAPSE_LIMIT;
    visibility
        .overflow_insertions
        .push((group[visible_start], visible_start));
    visibility
        .visible
        .extend_from_slice(&group[visible_start..]);
}

fn fold_state_hash(fold_state: &TranscriptFoldState) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    fold_state.schema_version.hash(&mut hasher);
    for (key, expanded) in &fold_state.expanded {
        key.hash(&mut hasher);
        expanded.hash(&mut hasher);
    }
    hasher.finish()
}

impl ToolTimelineTool {
    fn detail(&self) -> Option<ToolDetail> {
        if !is_shell_like_tool(&self.entry.name) {
            return None;
        }

        let (command, arguments) = command_and_arguments(&self.entry.arguments);
        Some(ToolDetail {
            tool_id: Some(self.tool_id.clone()),
            title: self.entry.label(),
            command,
            arguments,
            output: self.output.clone().filter(|text| !text.trim().is_empty()),
            failed: self.status == ToolTimelineStatus::Failed,
            running: self.status == ToolTimelineStatus::Running,
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

fn append_tool_output(output: &mut Option<String>, delta: &str) {
    if delta.trim() == "(no output)" && output.as_ref().is_some_and(|text| !text.trim().is_empty())
    {
        return;
    }
    let existing = output.get_or_insert_with(String::new);
    if !existing.is_empty() && !existing.ends_with('\n') {
        existing.push('\n');
    }
    existing.push_str(delta);
}

fn shift_indices_after_removal<K>(indices: &mut HashMap<K, usize>, removed_index: usize) {
    for index in indices.values_mut() {
        if *index > removed_index {
            *index -= 1;
        }
    }
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
    lines: &[Line<'static>],
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
        let line_height = line_visual_height(line, width);
        let line_end = line_start.saturating_add(line_height);
        visual_row = line_end;
        if line_end <= render_top {
            first_line_start = line_end;
            continue;
        }
        if line_start >= render_bottom {
            break;
        }
        window.push(line.clone());
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

fn item_is_non_message_selectable(item: &TimelineItem) -> bool {
    matches!(
        item.kind,
        TimelineItemKind::Tool(_)
            | TimelineItemKind::Shell(_)
            | TimelineItemKind::SubagentTrace(_)
            | TimelineItemKind::PlanReview(_)
    )
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
            | TimelineItemKind::System(text)
            | TimelineItemKind::Error(text)
            | TimelineItemKind::Shell(text)
            | TimelineItemKind::ShellOutput(text) => Some(text),
            TimelineItemKind::Assistant(message) => Some(&message.text),
            TimelineItemKind::Reasoning(message) => Some(&message.text),
            TimelineItemKind::Tool(_)
            | TimelineItemKind::SubagentTrace(_)
            | TimelineItemKind::PlanReview(_)
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
