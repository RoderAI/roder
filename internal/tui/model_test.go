package tui

import (
	"context"
	"errors"
	"strings"
	"testing"
	"time"

	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"
	"github.com/pandelisz/gode/internal/godex"
	"github.com/pandelisz/gode/internal/godex/agent"
	"github.com/pandelisz/gode/internal/godex/eventbus"
	messagestore "github.com/pandelisz/gode/internal/godex/message"
	"github.com/pandelisz/gode/internal/tui/dialogs"
	"github.com/pandelisz/gode/internal/tui/eventadapter"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

func TestModelAppendsAssistantDeltaEvents(t *testing.T) {
	app, err := godex.New(context.Background(), godex.Config{DataDir: t.TempDir(), Workspace: t.TempDir(), Provider: "mock", AutoApprove: true})
	if err != nil {
		t.Fatalf("app: %v", err)
	}
	defer app.Close(context.Background())

	model := New(app)
	updated, _ := model.Update(eventMsg{Event: eventbus.Event{Kind: eventbus.KindAssistantDelta, Payload: map[string]any{"text": "hello"}}})
	got := updated.(Model)
	if len(got.messages) != 1 || got.messages[0].Body != "hello" {
		t.Fatalf("messages = %#v", got.messages)
	}
}

func TestModelCoalescesAssistantDeltaEvents(t *testing.T) {
	model := New(nil)
	updated, _ := model.Update(eventMsg{Event: eventbus.Event{Kind: eventbus.KindAssistantDelta, Payload: map[string]any{"text": "hel"}}})
	updated, _ = updated.Update(eventMsg{Event: eventbus.Event{Kind: eventbus.KindAssistantDelta, Payload: map[string]any{"text": "lo"}}})

	got := updated.(Model)
	if len(got.messages) != 1 || got.messages[0].Body != "hello" {
		t.Fatalf("messages = %#v", got.messages)
	}
}

func TestModelKeepsCommentaryAndFinalAnswerDeltasSeparate(t *testing.T) {
	model := New(nil)
	updated, _ := model.Update(eventMsg{Event: eventbus.Event{Kind: eventbus.KindAssistantDelta, Payload: map[string]any{"text": "I will inspect first.", "phase": "commentary"}}})
	updated, _ = updated.Update(eventMsg{Event: eventbus.Event{Kind: eventbus.KindAssistantDelta, Payload: map[string]any{"text": "Done.", "phase": "final_answer"}}})

	got := updated.(Model)
	if len(got.messages) != 2 {
		t.Fatalf("messages = %#v", got.messages)
	}
	if got.messages[0].Title != "commentary" || got.messages[0].Body != "I will inspect first." {
		t.Fatalf("commentary message = %#v", got.messages[0])
	}
	if got.messages[1].Title != "" || got.messages[1].Body != "Done." {
		t.Fatalf("final message = %#v", got.messages[1])
	}
}

func TestModelDrainsBufferedFastEvents(t *testing.T) {
	app, err := godex.New(context.Background(), godex.Config{DataDir: t.TempDir(), Workspace: t.TempDir(), Provider: "mock", AutoApprove: true})
	if err != nil {
		t.Fatalf("app: %v", err)
	}
	defer app.Close(context.Background())

	model := New(app)
	defer model.cancelEvents()
	app.Bus.Publish(context.Background(), eventbus.Event{Kind: eventbus.KindAssistantDelta, Payload: map[string]any{"text": "hel"}})
	app.Bus.Publish(context.Background(), eventbus.Event{Kind: eventbus.KindAssistantDelta, Payload: map[string]any{"text": "lo"}})

	firstCmd := model.Init()
	if firstCmd == nil {
		t.Fatal("expected initial event wait command")
	}
	updated, nextCmd := model.Update(firstCmd())
	got := updated.(Model)
	if nextCmd == nil {
		t.Fatal("expected follow-up event wait command")
	}
	updated, _ = got.Update(nextCmd())
	got = updated.(Model)

	if len(got.messages) != 1 || got.messages[0].Body != "hello" {
		t.Fatalf("messages = %#v", got.messages)
	}
}

func TestModelSummarizesReadFileToolOutput(t *testing.T) {
	model := New(nil)
	fullContents := strings.Repeat("package main\n", 200)
	updated, _ := model.Update(eventMsg{Event: eventbus.Event{
		Kind: eventbus.KindToolCompleted,
		Payload: map[string]any{
			"tool": "read_file",
			"input": map[string]any{
				"path": "internal/godex/tools/registry.go",
			},
			"text": fullContents,
		},
	}})

	got := updated.(Model)
	if len(got.messages) != 1 {
		t.Fatalf("messages = %#v", got.messages)
	}
	if got.messages[0].Body != "read internal/godex/tools/registry.go" {
		t.Fatalf("tool body = %q", got.messages[0].Body)
	}
	if strings.Contains(got.messages[0].Body, "package main") {
		t.Fatalf("read_file timeline should not include file contents:\n%s", got.messages[0].Body)
	}
}

func TestModelUpdatesToolTimelineEntryForSameCall(t *testing.T) {
	model := New(nil)
	request := eventbus.Event{
		Kind: eventbus.KindToolRequested,
		Payload: map[string]any{
			"tool":         "read_file",
			"tool_call_id": "call_1",
			"input": map[string]any{
				"path": "README.md",
			},
		},
	}
	completed := eventbus.Event{
		Kind: eventbus.KindToolCompleted,
		Payload: map[string]any{
			"tool":         "read_file",
			"tool_call_id": "call_1",
			"input": map[string]any{
				"path": "README.md",
			},
			"text": strings.Repeat("contents\n", 20),
		},
	}

	updated, _ := model.Update(eventMsg{Event: request})
	updated, _ = updated.Update(eventMsg{Event: completed})
	got := updated.(Model)

	if len(got.messages) != 1 {
		t.Fatalf("tool request and completion should render one row: %#v", got.messages)
	}
	if got.messages[0].Role != viewmodel.RoleTool || got.messages[0].Title != "read_file" || got.messages[0].Body != "read README.md" {
		t.Fatalf("tool message = %#v", got.messages[0])
	}
	if strings.Contains(got.messages[0].Body, "contents") {
		t.Fatalf("read_file timeline leaked output:\n%s", got.messages[0].Body)
	}
}

func TestEventAdapterStateEventsDoNotRenderEmptyTranscriptRows(t *testing.T) {
	model := New(nil)
	events := []eventbus.Event{
		{Kind: eventbus.KindPermissionResponded, Payload: map[string]any{"decision": "deny"}},
		{Kind: eventbus.KindMCPStateChanged, Payload: map[string]any{"server": "github", "state": "connected"}},
		{Kind: eventbus.KindLSPStateChanged, Payload: map[string]any{"server": "gopls", "state": "connected"}},
		{Kind: eventadapter.KindHookResult, Payload: map[string]any{"hook": "policy", "decision": "allow"}},
		{Kind: eventadapter.KindSessionUpdate, Payload: map[string]any{"title": "feature"}},
		{Kind: eventadapter.KindModelChanged, Payload: map[string]any{"model": "gpt-5.5"}},
	}

	var updated tea.Model = model
	for _, ev := range events {
		updated, _ = updated.Update(eventMsg{Event: ev})
	}
	got := updated.(Model)
	if len(got.messages) != 0 {
		t.Fatalf("state events should not render transcript rows: %#v", got.messages)
	}
	if strings.TrimSpace(got.status) == "" {
		t.Fatal("state events should leave a useful status")
	}
}

func TestModelShowsReasoningSummaryAboveComposer(t *testing.T) {
	model := New(nil)
	model.width = 80
	model.height = 24
	model.running = true
	updated, _ := model.Update(eventMsg{Event: eventbus.Event{Kind: eventbus.KindReasoningSummaryDelta, Payload: map[string]any{"text": "Checking workspace"}}})
	updated, _ = updated.Update(eventMsg{Event: eventbus.Event{Kind: eventbus.KindReasoningSummaryDelta, Payload: map[string]any{"text": " before editing."}}})

	got := updated.(Model)
	if got.reasoningSummary != "Checking workspace before editing." {
		t.Fatalf("reasoning summary = %q", got.reasoningSummary)
	}
	view := got.View().Content
	reasoningIndex := strings.Index(view, "REASONING")
	inputIndex := strings.Index(view, "sk gode to work on this repo")
	if reasoningIndex < 0 || inputIndex < 0 || reasoningIndex > inputIndex {
		t.Fatalf("reasoning summary should render above composer:\n%s", view)
	}
	if !strings.Contains(view, "Checking workspace before editing.") {
		t.Fatalf("view missing reasoning summary:\n%s", view)
	}
}

func TestModelHidesReasoningSummaryWhenIdle(t *testing.T) {
	model := New(nil)
	model.width = 80
	model.height = 24
	model.reasoningSummary = "Finished checking workspace."
	model.running = false

	view := model.View().Content
	if strings.Contains(view, "REASONING") || strings.Contains(view, "Finished checking workspace.") {
		t.Fatalf("idle view should hide reasoning summary:\n%s", view)
	}
}

func TestModelShowsContextLeftInFooter(t *testing.T) {
	model := New(nil)
	model.width = 80
	model.height = 12

	updated, _ := model.Update(eventMsg{Event: eventbus.Event{
		Kind: eventbus.KindContextTokensUpdated,
		Payload: map[string]any{
			"tokens":         1200,
			"context_window": 10000,
			"percent":        12.4,
		},
	}})

	got := updated.(Model)
	if got.contextLeft != "ctx 88%" {
		t.Fatalf("context left = %q", got.contextLeft)
	}
	if !strings.Contains(got.View().Content, "ctx 88%") {
		t.Fatalf("view missing context left:\n%s", got.View().Content)
	}
}

func TestModelEnterWhileRunningSubmitsSteer(t *testing.T) {
	model := New(nil)
	model.running = true
	model.currentSessionID = "s1"
	model.input.SetValue("change direction")

	updated, cmd := model.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	got := updated.(Model)
	if cmd == nil {
		t.Fatal("expected steer command")
	}
	if strings.TrimSpace(got.input.Value()) != "" {
		t.Fatalf("input should clear after steer, got %q", got.input.Value())
	}
	if len(got.messages) != 1 || got.messages[0].Role != viewmodel.RoleUser || got.messages[0].Title != "steer" || got.messages[0].Body != "change direction" {
		t.Fatalf("messages = %#v", got.messages)
	}
	if got.status != "steer queued for active run" {
		t.Fatalf("status = %q", got.status)
	}
}

func TestModelTabWhileRunningQueuesPrompt(t *testing.T) {
	model := New(nil)
	model.running = true
	model.input.SetValue("run after this")

	updated, cmd := model.Update(tea.KeyPressMsg{Code: tea.KeyTab})
	got := updated.(Model)
	if cmd != nil {
		t.Fatal("queueing should not start a command immediately")
	}
	if len(got.queuedPrompts) != 1 || got.queuedPrompts[0].Prompt != "run after this" {
		t.Fatalf("queued prompts = %#v", got.queuedPrompts)
	}
	if strings.TrimSpace(got.input.Value()) != "" {
		t.Fatalf("input should clear after queue, got %q", got.input.Value())
	}
	if !strings.Contains(got.footerStatus(), "queued 1 prompt") {
		t.Fatalf("footer status = %q", got.footerStatus())
	}
	if !strings.Contains(got.View().Content, "Queued follow-up inputs") || !strings.Contains(got.View().Content, "run after this") {
		t.Fatalf("view missing queued prompt block:\n%s", got.View().Content)
	}
}

func TestModelRunDoneStartsNextQueuedPrompt(t *testing.T) {
	model := New(nil)
	model.currentSessionID = "s1"
	model.queuedPrompts = []pendingPrompt{{Display: "next", Prompt: "next"}}

	updated, cmd := model.Update(runDoneMsg{Result: agent.RunResult{SessionID: "s1", FinalText: "done"}})
	got := updated.(Model)
	if cmd == nil {
		t.Fatal("expected queued prompt command")
	}
	if len(got.queuedPrompts) != 0 || !got.running {
		t.Fatalf("queued=%#v running=%v", got.queuedPrompts, got.running)
	}
	if len(got.messages) != 1 || got.messages[0].Body != "next" {
		t.Fatalf("messages = %#v", got.messages)
	}
}

func TestFormatContextLeftClampsBounds(t *testing.T) {
	for _, tc := range []struct {
		used float64
		want string
	}{
		{used: -4.9, want: "ctx 100%"},
		{used: 0, want: "ctx 100%"},
		{used: 12.6, want: "ctx 87%"},
		{used: 101, want: "ctx 0%"},
	} {
		if got := formatContextLeft(tc.used); got != tc.want {
			t.Fatalf("formatContextLeft(%v) = %q, want %q", tc.used, got, tc.want)
		}
	}
}

func TestModelScrollState(t *testing.T) {
	model := New(nil)
	model.width = 40
	model.height = 10
	for i := 0; i < 10; i++ {
		model.addMessage("user", "", "message")
	}

	model.scrollBy(3)
	if model.scrollOffset != 3 || model.followTail {
		t.Fatalf("scrollOffset=%d followTail=%v", model.scrollOffset, model.followTail)
	}

	model.follow()
	if model.scrollOffset != 0 || !model.followTail {
		t.Fatalf("scrollOffset=%d followTail=%v", model.scrollOffset, model.followTail)
	}
}

func TestModelKeepsEntireTimelineHistory(t *testing.T) {
	model := New(nil)
	model.width = 80
	model.height = 20
	for i := 0; i < 550; i++ {
		model.addMessage("user", "", "message")
	}

	if len(model.messages) != 550 {
		t.Fatalf("messages retained = %d, want 550", len(model.messages))
	}
	model.scrollToOldest()
	if model.scrollOffset <= 0 {
		t.Fatalf("scrollOffset = %d, want ability to scroll to oldest history", model.scrollOffset)
	}
}

func TestNewModelFocusesComposer(t *testing.T) {
	model := New(nil)
	if !model.input.Focused() {
		t.Fatal("composer should be focused")
	}
}

func TestComposerDoesNotPaintCursorLineBackground(t *testing.T) {
	model := New(nil)
	view := model.input.View()
	if strings.Contains(view, "\x1b[40m") || strings.Contains(view, "\x1b[48;5;0m") {
		t.Fatalf("composer view contains black background ANSI: %q", view)
	}
}

func TestCtrlPOpensSettingsDialog(t *testing.T) {
	model := New(nil)
	updated, _ := model.Update(keyCtrlP())
	got := updated.(Model)

	if !got.settings.Open {
		t.Fatal("settings dialog should be open")
	}
	if got.input.Focused() {
		t.Fatal("composer should blur while settings dialog is open")
	}
	view := got.View().Content
	for _, want := range []string{"Settings", "Models", "Fast Mode", "Codex Sign In", "Config"} {
		if !strings.Contains(view, want) {
			t.Fatalf("view should render settings menu %q:\n%s", want, view)
		}
	}
}

func TestViewKeepsComposerAndFooterInsideWindow(t *testing.T) {
	model := New(nil)
	updated, _ := model.Update(tea.WindowSizeMsg{Width: 120, Height: 32})
	got := updated.(Model)
	got.messages = []viewmodel.Message{
		{ID: "m1", Role: viewmodel.RoleAssistant, Body: strings.Repeat("assistant text ", 80)},
	}
	got.status = "running"

	view := got.View().Content
	lines := strings.Split(view, "\n")
	if len(lines) != 32 {
		t.Fatalf("view line count = %d, want 32\n%s", len(lines), view)
	}
	for i, line := range lines {
		if lipgloss.Width(line) > 120 {
			t.Fatalf("line %d width %d exceeds viewport:\n%s", i, lipgloss.Width(line), view)
		}
	}
	if !strings.Contains(lines[len(lines)-2], "└") {
		t.Fatalf("composer bottom border should remain visible, got %q\n%s", lines[len(lines)-2], view)
	}
	if !strings.Contains(lines[len(lines)-1], "scroll") {
		t.Fatalf("footer should remain visible on bottom row, got %q\n%s", lines[len(lines)-1], view)
	}
}

func TestSlashShowsInlineMenuAndSelectionInsertsCommand(t *testing.T) {
	model := New(nil)
	updated, _ := model.Update(tea.KeyPressMsg{Code: '/', Text: "/"})
	got := updated.(Model)
	if got.commands.Open {
		t.Fatal("slash key should use the inline menu, not the modal command dialog")
	}
	if got.input.Value() != "/" {
		t.Fatalf("input = %q", got.input.Value())
	}
	view := got.View().Content
	for _, want := range []string{"/goal", "set, show, pause"} {
		if !strings.Contains(view, want) {
			t.Fatalf("slash menu missing %q:\n%s", want, view)
		}
	}

	updated, _ = got.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	got = updated.(Model)
	if got.commands.Open {
		t.Fatal("commands dialog should stay closed after inline selection")
	}
	if got.input.Value() != "/goal " {
		t.Fatalf("input = %q", got.input.Value())
	}
	if !got.input.Focused() {
		t.Fatal("composer should stay focused after inline command selection")
	}
}

func TestSlashMenuFiltersProjectCommands(t *testing.T) {
	app, err := godex.New(context.Background(), godex.Config{DataDir: t.TempDir(), Workspace: t.TempDir(), Provider: "mock", AutoApprove: true})
	if err != nil {
		t.Fatalf("app: %v", err)
	}
	defer app.Close(context.Background())
	model := New(app)
	model.input.SetValue("/go")

	menu := model.slashMenuViewModel()
	if menu == nil || len(menu.Items) != 1 || menu.Items[0].ID != "goal" {
		t.Fatalf("slash menu = %#v", menu)
	}
}

func TestGoalCommandCreatesGoalAndStartsModelTurn(t *testing.T) {
	app, err := godex.New(context.Background(), godex.Config{DataDir: t.TempDir(), Workspace: t.TempDir(), Provider: "mock", AutoApprove: true})
	if err != nil {
		t.Fatalf("app: %v", err)
	}
	defer app.Close(context.Background())
	model := New(app)
	defer model.cancelEvents()
	model.input.SetValue("/goal ship the release")

	updated, cmd := model.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	got := updated.(Model)
	if cmd == nil {
		t.Fatal("goal command should submit a model turn")
	}
	if !got.running {
		t.Fatal("goal command should start a run")
	}
	goal, err := app.GetGoal(context.Background(), got.currentSessionID)
	if err != nil {
		t.Fatalf("get goal: %v", err)
	}
	if goal == nil || goal.Objective != "ship the release" {
		t.Fatalf("goal = %#v", goal)
	}
	if len(got.messages) != 2 || got.messages[0].Role != viewmodel.RoleSystem || got.messages[1].Role != viewmodel.RoleUser || !strings.Contains(got.messages[0].Body, "ship the release") || got.messages[1].Body != "ship the release" {
		t.Fatalf("messages = %#v", got.messages)
	}
	if !strings.Contains(got.View().Content, "goal active") {
		t.Fatalf("view missing goal footer:\n%s", got.View().Content)
	}
}

func TestGoalPauseResumeAndClearCommands(t *testing.T) {
	app, err := godex.New(context.Background(), godex.Config{DataDir: t.TempDir(), Workspace: t.TempDir(), Provider: "mock", AutoApprove: true})
	if err != nil {
		t.Fatalf("app: %v", err)
	}
	defer app.Close(context.Background())
	var updated tea.Model = New(app)
	got := updated.(Model)
	defer got.cancelEvents()
	got.input.SetValue("/goal ship")
	updated, _ = got.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	got = updated.(Model)
	got.running = false
	sessionID := got.currentSessionID

	for _, input := range []string{"/goal pause", "/goal resume", "/goal budget 50000"} {
		got.input.SetValue(input)
		updated, _ = got.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
		got = updated.(Model)
	}
	goal, err := app.GetGoal(context.Background(), sessionID)
	if err != nil {
		t.Fatalf("get goal: %v", err)
	}
	if goal == nil || goal.Status != "active" || goal.TokenBudget == nil || *goal.TokenBudget != 50000 {
		t.Fatalf("goal = %#v", goal)
	}
	got.input.SetValue("/goal clear")
	updated, _ = got.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	got = updated.(Model)
	goal, err = app.GetGoal(context.Background(), sessionID)
	if err != nil {
		t.Fatalf("get cleared goal: %v", err)
	}
	if goal != nil {
		t.Fatalf("goal should be cleared: %#v", goal)
	}
}

func TestAbsolutePathDoesNotOpenSlashCommandDialog(t *testing.T) {
	model := New(nil)
	for _, key := range []string{"/", "U", "s", "e", "r", "s"} {
		updated, _ := model.Update(tea.KeyPressMsg{Code: []rune(key)[0], Text: key})
		model = updated.(Model)
	}
	if model.commands.Open {
		t.Fatal("absolute path text should not open commands dialog")
	}
	if model.input.Value() != "/Users" {
		t.Fatalf("input = %q", model.input.Value())
	}
}

func TestEscapeClosesCommandsDialog(t *testing.T) {
	model := New(nil)
	model.commands = dialogs.NewCommands([]dialogs.CommandItem{{ID: "project:test", Title: "/project:test"}})

	updated, _ := model.Update(tea.KeyPressMsg{Code: tea.KeyEscape})
	got := updated.(Model)
	if got.commands.Open {
		t.Fatal("commands dialog should close on escape")
	}
	if !got.input.Focused() {
		t.Fatal("composer should refocus after closing commands")
	}
}

func TestCtrlSOpensSessionsDialogAndLoadsMessages(t *testing.T) {
	app, err := godex.New(context.Background(), godex.Config{DataDir: t.TempDir(), Workspace: t.TempDir(), Provider: "mock", AutoApprove: true})
	if err != nil {
		t.Fatalf("app: %v", err)
	}
	defer app.Close(context.Background())
	session, err := app.Sessions.Create(context.Background(), "Earlier work", "")
	if err != nil {
		t.Fatalf("create session: %v", err)
	}
	if _, err := app.Messages.Append(context.Background(), messagestore.Message{SessionID: session.ID, Role: messagestore.RoleUser, Text: "hello"}); err != nil {
		t.Fatalf("append user: %v", err)
	}
	if _, err := app.Messages.Append(context.Background(), messagestore.Message{SessionID: session.ID, Role: messagestore.RoleAssistant, Text: "world"}); err != nil {
		t.Fatalf("append assistant: %v", err)
	}

	model := New(app)
	defer model.cancelEvents()
	updated, _ := model.Update(tea.KeyPressMsg{Code: 's', Mod: tea.ModCtrl})
	got := updated.(Model)
	if !got.sessions.Open {
		t.Fatal("sessions dialog should open")
	}
	if len(got.sessions.Items) != 2 || got.sessions.Items[0].ID != dialogs.NewSessionID || got.sessions.Items[1].ID != session.ID {
		t.Fatalf("session items = %#v", got.sessions.Items)
	}

	got.sessions.Move(1)
	updated, _ = got.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	got = updated.(Model)
	if got.currentSessionID != session.ID {
		t.Fatalf("current session = %q", got.currentSessionID)
	}
	if got.currentSession != "Earlier work" {
		t.Fatalf("current session title = %q", got.currentSession)
	}
	if len(got.messages) != 2 || got.messages[0].Body != "hello" || got.messages[1].Body != "world" {
		t.Fatalf("messages = %#v", got.messages)
	}
}

func TestNewSessionLoadsSelectedSession(t *testing.T) {
	app, err := godex.New(context.Background(), godex.Config{DataDir: t.TempDir(), Workspace: t.TempDir(), Provider: "mock", AutoApprove: true})
	if err != nil {
		t.Fatalf("app: %v", err)
	}
	defer app.Close(context.Background())
	session, err := app.Sessions.Create(context.Background(), "Recent session", "")
	if err != nil {
		t.Fatalf("create session: %v", err)
	}
	if _, err := app.Messages.Append(context.Background(), messagestore.Message{SessionID: session.ID, Role: messagestore.RoleUser, Text: "resume me"}); err != nil {
		t.Fatalf("append user: %v", err)
	}

	model, err := NewSession(app, session.ID)
	if err != nil {
		t.Fatalf("new session model: %v", err)
	}
	defer model.cancelEvents()
	if model.currentSessionID != session.ID || model.currentSession != "Recent session" {
		t.Fatalf("current session = %q title %q", model.currentSessionID, model.currentSession)
	}
	if len(model.messages) != 1 || model.messages[0].Body != "resume me" {
		t.Fatalf("messages = %#v", model.messages)
	}
}

func TestNewSessionActionClearsViewWithoutDeletingHistory(t *testing.T) {
	app, err := godex.New(context.Background(), godex.Config{DataDir: t.TempDir(), Workspace: t.TempDir(), Provider: "mock", AutoApprove: true})
	if err != nil {
		t.Fatalf("app: %v", err)
	}
	defer app.Close(context.Background())
	session, err := app.Sessions.Create(context.Background(), "Earlier work", "")
	if err != nil {
		t.Fatalf("create session: %v", err)
	}

	model := New(app)
	defer model.cancelEvents()
	model.currentSessionID = session.ID
	model.currentSession = "Earlier work"
	model.input.SetValue("draft prompt")
	model.addMessage(viewmodel.RoleUser, "", "hello")
	model.sessions = dialogs.NewSessions(model.sessionItems())

	updated, _ := model.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	got := updated.(Model)
	if got.currentSessionID != "" || got.currentSession != "" {
		t.Fatalf("current session = %q title %q", got.currentSessionID, got.currentSession)
	}
	if len(got.messages) != 0 || strings.TrimSpace(got.input.Value()) != "" {
		t.Fatalf("view not cleared: messages=%#v input=%q", got.messages, got.input.Value())
	}
	sessions, err := app.Sessions.List(context.Background())
	if err != nil {
		t.Fatalf("list sessions: %v", err)
	}
	if len(sessions) != 1 || sessions[0].ID != session.ID {
		t.Fatalf("history was deleted: %#v", sessions)
	}
}

func TestPermissionDialogRespondsOnEventBus(t *testing.T) {
	app, err := godex.New(context.Background(), godex.Config{DataDir: t.TempDir(), Workspace: t.TempDir(), Provider: "mock", AutoApprove: true})
	if err != nil {
		t.Fatalf("app: %v", err)
	}
	defer app.Close(context.Background())
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	responses := app.Bus.Subscribe(ctx, eventbus.Filter{CorrelationID: "corr-1", Kinds: []eventbus.Kind{eventbus.KindPermissionResponded}})

	model := New(app)
	defer model.cancelEvents()
	updated, _ := model.Update(eventMsg{Event: eventbus.Event{
		Kind:          eventbus.KindPermissionRequested,
		SessionID:     "s1",
		RunID:         "r1",
		CorrelationID: "corr-1",
		Payload: map[string]any{
			"tool":   "write_file",
			"action": "write",
			"path":   "README.md",
		},
	}})
	got := updated.(Model)
	if !got.permissions.Open || len(got.permissions.Requests) != 1 {
		t.Fatalf("permissions = %#v", got.permissions)
	}

	updated, _ = got.Update(tea.KeyPressMsg{Code: 's', Text: "s"})
	got = updated.(Model)
	if got.permissions.Open {
		t.Fatal("permission dialog should close after responding")
	}
	select {
	case ev := <-responses:
		var payload struct {
			Approved        bool `json:"approved"`
			AllowForSession bool `json:"allow_for_session"`
		}
		if err := ev.DecodePayload(&payload); err != nil {
			t.Fatalf("decode response: %v", err)
		}
		if !payload.Approved || !payload.AllowForSession {
			t.Fatalf("payload = %#v", payload)
		}
	case <-time.After(time.Second):
		t.Fatal("timed out waiting for permission response")
	}
}

func TestCtrlLTogglesErrorLogBelowComposer(t *testing.T) {
	model := New(nil)
	updated, _ := model.Update(runDoneMsg{Err: errors.New(`POST "https://chatgpt.com/backend-api/codex/responses": 400 Bad Request`)})
	got := updated.(Model)

	if len(got.errorLog) != 1 {
		t.Fatalf("error log length = %d", len(got.errorLog))
	}
	if got.showErrorLog {
		t.Fatal("error log should stay hidden until ctrl+l")
	}
	if got.status != "run failed - ctrl+l errors" {
		t.Fatalf("status = %q", got.status)
	}

	updated, _ = got.Update(keyCtrlL())
	got = updated.(Model)
	if !got.showErrorLog {
		t.Fatal("error log should be visible")
	}
	view := got.View().Content
	for _, want := range []string{"ERROR LOG", "400 Bad Request"} {
		if !strings.Contains(view, want) {
			t.Fatalf("view should render error log %q:\n%s", want, view)
		}
	}
}

func TestRunDoneDoesNotDuplicateRunFailedEventError(t *testing.T) {
	const message = `POST "https://chatgpt.com/backend-api/codex/responses": 400 Bad Request`
	model := New(nil)
	updated, _ := model.Update(eventMsg{Event: eventbus.Event{Kind: eventbus.KindRunFailed, Payload: map[string]any{"error": message}}})
	updated, _ = updated.Update(runDoneMsg{Err: errors.New(message)})
	got := updated.(Model)

	if len(got.messages) != 1 {
		t.Fatalf("messages length = %d, want 1: %#v", len(got.messages), got.messages)
	}
	if len(got.errorLog) != 1 {
		t.Fatalf("error log length = %d, want 1: %#v", len(got.errorLog), got.errorLog)
	}
}

func TestRunFailedEventUsesDetailInErrorLog(t *testing.T) {
	detail := strings.Join([]string{
		"agent stopped without final text",
		"",
		"debug:",
		"session_id: s1",
		"run_id: r1",
		"last_tool: shell",
	}, "\n")
	model := New(nil)
	updated, _ := model.Update(eventMsg{Event: eventbus.Event{Kind: eventbus.KindRunFailed, Payload: map[string]any{
		"error":  "agent stopped without final text",
		"detail": detail,
	}}})
	got := updated.(Model)

	if len(got.messages) != 1 {
		t.Fatalf("messages length = %d", len(got.messages))
	}
	if strings.Contains(got.messages[0].Body, "last_tool") {
		t.Fatalf("timeline should stay summarized:\n%s", got.messages[0].Body)
	}
	if len(got.errorLog) != 1 || got.errorLog[0].Message != detail {
		t.Fatalf("error log = %#v", got.errorLog)
	}
}

func keyCtrlL() tea.KeyPressMsg {
	return tea.KeyPressMsg{Code: 'l', Mod: tea.ModCtrl}
}
