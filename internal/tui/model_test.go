package tui

import (
	"context"
	"errors"
	"strings"
	"testing"
	"time"

	tea "charm.land/bubbletea/v2"
	"github.com/pandelisz/gode/internal/godex"
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

func TestSlashOpensCommandsDialogAndSelectionInsertsCommand(t *testing.T) {
	model := New(nil)
	updated, _ := model.Update(tea.KeyPressMsg{Code: '/', Text: "/"})
	got := updated.(Model)
	if got.commands.Open {
		t.Fatal("slash key should stay in the composer until it is submitted bare")
	}
	if got.input.Value() != "/" {
		t.Fatalf("input = %q", got.input.Value())
	}

	updated, _ = got.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	got = updated.(Model)
	if !got.commands.Open {
		t.Fatal("commands dialog should open when bare slash is submitted")
	}

	got.commands = dialogs.NewCommands([]dialogs.CommandItem{{ID: "project:test", Title: "/project:test", Source: "project"}})
	updated, _ = got.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	got = updated.(Model)

	if got.commands.Open {
		t.Fatal("commands dialog should close after selection")
	}
	if got.input.Value() != "/project:test " {
		t.Fatalf("input = %q", got.input.Value())
	}
	if !got.input.Focused() {
		t.Fatal("composer should refocus after command selection")
	}
}

func TestGoalCommandCreatesGoalWithoutSubmittingPrompt(t *testing.T) {
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
	if cmd != nil {
		t.Fatal("goal command should not submit a model turn")
	}
	if got.running {
		t.Fatal("goal command should not start a run")
	}
	goal, err := app.GetGoal(context.Background(), got.currentSessionID)
	if err != nil {
		t.Fatalf("get goal: %v", err)
	}
	if goal == nil || goal.Objective != "ship the release" {
		t.Fatalf("goal = %#v", goal)
	}
	if len(got.messages) != 1 || got.messages[0].Role != viewmodel.RoleSystem || !strings.Contains(got.messages[0].Body, "ship the release") {
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
		"agent stopped without final text after tool loop",
		"",
		"debug:",
		"session_id: s1",
		"run_id: r1",
		"max_turns: 8",
		"last_tool: shell",
	}, "\n")
	model := New(nil)
	updated, _ := model.Update(eventMsg{Event: eventbus.Event{Kind: eventbus.KindRunFailed, Payload: map[string]any{
		"error":  "agent stopped without final text after tool loop",
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
