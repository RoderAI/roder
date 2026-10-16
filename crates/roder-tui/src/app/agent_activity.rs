use super::*;
use std::collections::BTreeMap;

struct AgentActivity {
    label: String,
    status: &'static str,
    timeline: TimelineState,
}

fn member_status(status: roder_api::teams::TeamMemberStatus) -> &'static str {
    use roder_api::teams::TeamMemberStatus;
    match status {
        TeamMemberStatus::Idle => "idle",
        TeamMemberStatus::Running => "running",
        TeamMemberStatus::Blocked => "blocked",
        TeamMemberStatus::Completed => "completed",
        TeamMemberStatus::Failed => "failed",
        TeamMemberStatus::Interrupted => "interrupted",
        TeamMemberStatus::Closed => "closed",
    }
}

fn record_timeline_event(timeline: &mut TimelineState, event: &RoderEvent) {
    use roder_api::inference::InferenceEvent;
    match event {
        RoderEvent::TurnStarted(_) => {
            timeline.flush_streaming_animation();
            timeline.push_system("Turn started");
        }
        RoderEvent::InferenceEventReceived(ev) => match &ev.event {
            InferenceEvent::MessageDelta(delta) => {
                timeline.push_assistant_delta_streaming(&delta.text, delta.phase.clone());
            }
            InferenceEvent::ReasoningDelta(delta) => {
                timeline.push_reasoning_delta_streaming(&delta.text);
            }
            InferenceEvent::HostedToolCallStarted(call) => {
                timeline.record_tool_requested(call.id.clone(), fallback_entry(call.name.clone()));
            }
            InferenceEvent::HostedToolCallCompleted(call) => {
                timeline.record_tool_requested(
                    call.id.clone(),
                    ToolTimelineEntry::new(call.name.clone(), call.arguments.clone()),
                );
                timeline.record_tool_completed(&call.id, false, None);
            }
            _ => {}
        },
        RoderEvent::ToolCallRequested(ev) => {
            timeline.record_tool_requested(
                ev.tool_id.clone(),
                tool_entry_from_display_payload(
                    Some(ev.tool_name.clone()),
                    ev.display_payload.clone(),
                ),
            );
        }
        RoderEvent::ToolCallCompleted(ev) => {
            timeline.record_tool_completed(&ev.tool_id, ev.is_error, ev.output.clone());
        }
        RoderEvent::ToolOutputDelta(ev) => {
            timeline.record_tool_output_delta(&ev.tool_id, &ev.delta);
        }
        RoderEvent::ApprovalRequested(ev) => {
            timeline.record_tool_requested(
                ev.tool_id.clone(),
                fallback_entry(format!("{} needs approval", ev.tool_name)),
            );
        }
        RoderEvent::ApprovalResolved(ev) if !ev.approved => {
            timeline.record_tool_completed(&ev.tool_id, true, None);
        }
        RoderEvent::TurnCompleted(_)
        | RoderEvent::TurnInterrupted(_)
        | RoderEvent::SubagentCompleted(_)
        | RoderEvent::TeamMemberCompleted(_) => {
            timeline.flush_streaming_animation();
        }
        RoderEvent::TurnFailed(ev) => {
            timeline.flush_streaming_animation();
            timeline.push_error(ev.error.clone());
        }
        RoderEvent::SubagentFailed(ev) => {
            timeline.flush_streaming_animation();
            timeline.push_error(ev.error.clone());
        }
        _ => {}
    }
}

/// Background streams never share the foreground transcript or turn counters.
#[derive(Default)]
pub(super) struct AgentActivityState {
    agents: BTreeMap<String, AgentActivity>,
    order: Vec<String>,
    open: bool,
    selected: usize,
    inspecting: Option<String>,
}

impl AgentActivityState {
    pub fn record(
        &mut self,
        event: &RoderEvent,
        scroll: ScrollSettings,
        settings: TimelineSettings,
    ) {
        let Some(thread_id) = event.thread_id() else {
            return;
        };
        if !self.agents.contains_key(thread_id) {
            self.order.push(thread_id.clone());
        }
        let agent = self
            .agents
            .entry(thread_id.clone())
            .or_insert_with(|| AgentActivity {
                label: short_id(thread_id).to_string(),
                status: "idle",
                timeline: TimelineState::new(scroll, settings),
            });
        record_timeline_event(&mut agent.timeline, event);
        match event {
            RoderEvent::TeamMemberStarted(ev) => {
                agent.label = ev
                    .member
                    .agent_path
                    .clone()
                    .unwrap_or_else(|| ev.member.name.clone());
                agent.status = member_status(ev.member.status);
            }
            RoderEvent::TeamMemberStatusChanged(ev) => {
                agent.status = member_status(ev.status);
            }
            RoderEvent::TeamMemberCompleted(ev) => {
                agent.status = member_status(ev.status);
            }
            RoderEvent::SubagentStarted(ev) => {
                agent.label = format!("{}: {}", ev.agent_type, ev.description);
                agent.status = "running";
            }
            RoderEvent::TurnStarted(_) => {
                agent.status = "running";
            }
            RoderEvent::InferenceEventReceived(_) | RoderEvent::ApprovalResolved(_) => {
                agent.status = "running";
            }
            RoderEvent::ApprovalRequested(_) | RoderEvent::UserInputRequested(_) => {
                agent.status = "blocked";
            }
            RoderEvent::UserInputResolved(_) => {
                agent.status = "running";
            }
            RoderEvent::TurnCompleted(_) | RoderEvent::SubagentCompleted(_) => {
                agent.status = "completed";
            }
            RoderEvent::TurnInterrupted(_) => {
                agent.status = "interrupted";
            }
            RoderEvent::TurnFailed(_) | RoderEvent::SubagentFailed(_) => {
                agent.status = "failed";
            }
            _ => {}
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn count(&self) -> usize {
        self.agents.len()
    }

    pub fn has_visible_animation(&self) -> bool {
        self.open
            && self
                .inspecting
                .as_ref()
                .and_then(|id| self.agents.get(id))
                .is_some_and(|agent| agent.timeline.has_streaming_animation())
    }

    pub fn open(&mut self) {
        self.open = true;
        self.inspecting = None;
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if matches!(key.code, KeyCode::Esc | KeyCode::Left) {
            if self.inspecting.take().is_none() {
                self.open = false;
            }
            return;
        }
        if let Some(id) = &self.inspecting {
            if let Some(agent) = self.agents.get_mut(id) {
                agent.timeline.handle_key(key);
            }
            return;
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = (self.selected + 1).min(self.agents.len().saturating_sub(1));
            }
            KeyCode::Enter | KeyCode::Right => {
                self.inspecting = self.order.get(self.selected).cloned();
                if let Some(agent) = self
                    .inspecting
                    .as_ref()
                    .and_then(|id| self.agents.get_mut(id))
                {
                    agent.timeline.focus_latest();
                }
            }
            _ => {}
        }
    }

    pub fn render(&mut self, f: &mut Frame<'_>, area: Rect, theme: Theme, animation_frame: u64) {
        f.render_widget(Clear, area);
        let chunks = Layout::vertical([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);
        let title = self
            .inspecting
            .as_ref()
            .and_then(|id| self.agents.get(id))
            .map(|agent| format!(" Agent: {} [{}]", agent.label, agent.status))
            .unwrap_or_else(|| format!(" Agents ({})", self.agents.len()));
        f.render_widget(Paragraph::new(title).style(theme.accent()), chunks[0]);
        if let Some(id) = &self.inspecting {
            if let Some(agent) = self.agents.get_mut(id) {
                // Only the visible stream needs animation work.
                agent
                    .timeline
                    .tick_streaming_animation(Instant::now(), chunks[1].width);
                let rendered = agent
                    .timeline
                    .render_with_frame(theme, chunks[1], animation_frame);
                f.render_widget(
                    Paragraph::new(rendered.text)
                        .style(theme.text())
                        .scroll((rendered.text_scroll, 0))
                        .wrap(Wrap { trim: false }),
                    chunks[1],
                );
            }
        } else if self.agents.is_empty() {
            f.render_widget(
                Paragraph::new(" No background agents yet.").style(theme.muted()),
                chunks[1],
            );
        } else {
            // Discovery order keeps the selection stable as new agents arrive.
            let items: Vec<ListItem<'_>> = self
                .order
                .iter()
                .map(|id| {
                    let agent = &self.agents[id];
                    let label = agent.label.split_whitespace().collect::<Vec<_>>().join(" ");
                    ListItem::new(format!(" {}  [{}]  {}", label, agent.status, short_id(id)))
                })
                .collect();
            let mut state = ListState::default().with_selected(Some(self.selected));
            f.render_stateful_widget(
                List::new(items)
                    .style(theme.text())
                    .highlight_style(theme.selected())
                    .highlight_symbol("> "),
                chunks[1],
                &mut state,
            );
        }
        f.render_widget(
            Paragraph::new(" up/down select  enter/right inspect  left/esc back  (read-only)")
                .style(theme.muted()),
            chunks[2],
        );
    }
}

impl<C: AppClient> TuiApp<C> {
    /// Return true for events consumed by a background stream. Approval and
    /// input requests still reach the canonical dialogs, regardless of owner.
    pub(super) fn route_background_agent_event(&mut self, event: &RoderEvent) -> bool {
        let Some(thread_id) = event.thread_id() else {
            return false;
        };
        if thread_id == &self.thread_id {
            return false;
        }
        match event {
            RoderEvent::InferenceEventReceived(_) => {
                self.team_ui.record_thread_activity(thread_id);
            }
            RoderEvent::TeamMemberStatusChanged(ev) => {
                self.team_ui.set_member_status(&ev.member_id, ev.status);
            }
            RoderEvent::TeamMemberCompleted(ev) => {
                self.team_ui.set_member_status(&ev.member_id, ev.status);
            }
            _ => {}
        }
        if let Some(timeline) = self.team_timeline_for_thread_mut(thread_id) {
            record_timeline_event(timeline, event);
        }
        self.agent_activity
            .record(event, self.scroll_settings, self.timeline_settings);
        !matches!(
            event,
            RoderEvent::ApprovalRequested(_)
                | RoderEvent::ApprovalResolved(_)
                | RoderEvent::UserInputRequested(_)
                | RoderEvent::UserInputResolved(_)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};
    use roder_api::{
        events::{InferenceEventReceived, TurnStarted},
        inference::{InferenceEvent, ReasoningDelta},
    };
    use time::OffsetDateTime;

    fn reasoning(thread: &str, text: &str) -> RoderEvent {
        RoderEvent::InferenceEventReceived(InferenceEventReceived {
            thread_id: thread.to_string(),
            turn_id: format!("{thread}-turn"),
            event: InferenceEvent::ReasoningDelta(ReasoningDelta { text: text.into() }),
            timestamp: OffsetDateTime::now_utc(),
        })
    }

    fn record(state: &mut AgentActivityState, event: &RoderEvent) {
        state.record(
            event,
            ScrollSettings::default(),
            TimelineSettings::default(),
        );
    }

    fn screen(state: &mut AgentActivityState, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|f| state.render(f, f.area(), Theme::for_terminal(), 0))
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn interleaved_reasoning_keeps_independent_streams() {
        let mut state = AgentActivityState::default();
        record(&mut state, &reasoning("agent-a", "ALPHA "));
        record(&mut state, &reasoning("agent-b", "BETA "));
        record(&mut state, &reasoning("agent-a", "CONTINUED"));
        record(&mut state, &reasoning("agent-b", "FINISHED"));
        for agent in state.agents.values_mut() {
            agent.timeline.flush_streaming_animation();
        }
        state.open();
        state.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let rendered = screen(&mut state, 80, 20);
        assert!(rendered.contains("ALPHA CONTINUED"), "{rendered}");
        assert!(!rendered.contains("BETA"));
        state.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        state.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        state.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let rendered = screen(&mut state, 80, 20);
        assert!(rendered.contains("BETA FINISHED"), "{rendered}");
        assert!(!rendered.contains("ALPHA"));
    }

    #[test]
    fn reused_agent_turns_do_not_concatenate() {
        let mut state = AgentActivityState::default();
        for (turn, text) in [("first", "FIRST RESPONSE"), ("second", "SECOND RESPONSE")] {
            record(
                &mut state,
                &RoderEvent::TurnStarted(TurnStarted {
                    thread_id: "reused-agent".to_string(),
                    turn_id: turn.to_string(),
                    runtime_profile: Default::default(),
                    timestamp: OffsetDateTime::now_utc(),
                }),
            );
            record(
                &mut state,
                &RoderEvent::InferenceEventReceived(InferenceEventReceived {
                    thread_id: "reused-agent".to_string(),
                    turn_id: turn.to_string(),
                    event: InferenceEvent::MessageDelta(roder_api::inference::MessageDelta {
                        text: text.to_string(),
                        phase: None,
                    }),
                    timestamp: OffsetDateTime::now_utc(),
                }),
            );
            record(
                &mut state,
                &RoderEvent::TurnCompleted(roder_api::events::TurnCompleted {
                    thread_id: "reused-agent".to_string(),
                    turn_id: turn.to_string(),
                    usage: None,
                    finish_reason: None,
                    timestamp: OffsetDateTime::now_utc(),
                }),
            );
        }
        state.open();
        state.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let rendered = screen(&mut state, 80, 25);
        assert_eq!(rendered.matches("Turn started").count(), 2, "{rendered}");
        assert!(rendered.contains("FIRST RESPONSE"), "{rendered}");
        assert!(rendered.contains("SECOND RESPONSE"), "{rendered}");
        assert!(!rendered.contains("FIRST RESPONSESECOND RESPONSE"));
    }

    #[test]
    fn hosted_tool_events_are_kept_in_the_owning_stream() {
        use roder_api::inference::{HostedToolCallCompleted, HostedToolCallStarted};
        let mut state = AgentActivityState::default();
        for event in [
            InferenceEvent::HostedToolCallStarted(HostedToolCallStarted {
                id: "lookup".into(),
                name: "web_search".into(),
            }),
            InferenceEvent::HostedToolCallCompleted(HostedToolCallCompleted {
                id: "lookup".into(),
                name: "web_search".into(),
                arguments: "example query".into(),
            }),
        ] {
            record(
                &mut state,
                &RoderEvent::InferenceEventReceived(InferenceEventReceived {
                    thread_id: "search-agent".into(),
                    turn_id: "search-turn".into(),
                    event,
                    timestamp: OffsetDateTime::now_utc(),
                }),
            );
        }
        state.open();
        state.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let rendered = screen(&mut state, 80, 20);
        assert!(rendered.to_ascii_lowercase().contains("web"), "{rendered}");
        assert!(rendered.contains('\u{2714}'), "{rendered}");
        assert_eq!(state.count(), 1);
    }

    #[test]
    fn overview_selection_is_full_width_in_light_and_dark_themes() {
        for dark in [false, true] {
            let theme = Theme::for_dark_background(dark);
            let mut state = AgentActivityState::default();
            record(&mut state, &reasoning("agent", "working"));
            state.open();
            let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
            terminal
                .draw(|f| state.render(f, f.area(), theme, 0))
                .unwrap();
            let buffer = terminal.backend().buffer();
            assert_eq!(buffer[(79, 2)].bg, theme.selection_bg);
            assert_eq!(buffer[(3, 2)].fg, theme.selection_fg);
        }
    }

    #[test]
    fn selection_does_not_jump_when_agents_arrive() {
        let mut state = AgentActivityState::default();
        record(&mut state, &reasoning("z-first", "first"));
        state.open();
        record(&mut state, &reasoning("a-second", "second"));
        state.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(state.inspecting.as_deref(), Some("z-first"));
        state.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(state.is_open());
        state.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!state.is_open());
    }

    #[test]
    fn overview_is_bounded_and_selected_agent_stays_visible() {
        let mut state = AgentActivityState::default();
        for index in 0..60 {
            record(
                &mut state,
                &RoderEvent::TurnStarted(TurnStarted {
                    thread_id: format!("{index:08}-agent"),
                    turn_id: format!("turn-{index}"),
                    runtime_profile: Default::default(),
                    timestamp: OffsetDateTime::now_utc(),
                }),
            );
        }
        state.open();
        for _ in 0..59 {
            state.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        }
        let rendered = screen(&mut state, 80, 12);
        assert!(rendered.contains("Agents (60)"));
        assert!(rendered.contains(">  00000059"), "{rendered}");
        assert!(rendered.contains("left/esc back"));
        // Very small terminals should clip safely without layout panics.
        screen(&mut state, 12, 3);
    }
}
