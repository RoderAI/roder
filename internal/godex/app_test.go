package godex

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/godex/goals"
	"github.com/pandelisz/gode/internal/godex/journal"
	messagestore "github.com/pandelisz/gode/internal/godex/message"
	"github.com/pandelisz/gode/internal/godex/provider"
	"github.com/pandelisz/gode/internal/godex/session"
)

func TestNewAppWiresBroadCoreWithMockProvider(t *testing.T) {
	app, err := New(context.Background(), Config{
		Workspace:   filepath.Join(t.TempDir(), "workspace"),
		DataDir:     t.TempDir(),
		Provider:    "mock",
		AutoApprove: true,
	})
	if err != nil {
		t.Fatalf("new app: %v", err)
	}
	defer app.Close(context.Background())

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	events := app.Bus.Subscribe(ctx, eventbus.Filter{Kinds: []eventbus.Kind{eventbus.KindRunCompleted}})

	result, err := app.RunPrompt(context.Background(), "hello")
	if err != nil {
		t.Fatalf("run: %v", err)
	}

	select {
	case ev := <-events:
		if ev.Kind != eventbus.KindRunCompleted {
			t.Fatalf("kind = %q", ev.Kind)
		}
	default:
		t.Fatal("expected run completed event")
	}

	messages, err := app.Messages.ListBySession(context.Background(), result.SessionID)
	if err != nil {
		t.Fatalf("messages: %v", err)
	}
	if len(messages) != 2 || messages[0].Text != "hello" || messages[1].Text != "mock response" {
		t.Fatalf("messages = %#v", messages)
	}
	session, ok, err := app.Sessions.Get(context.Background(), result.SessionID)
	if err != nil {
		t.Fatalf("session: %v", err)
	}
	if !ok || session.MessageCount != 2 {
		t.Fatalf("session = %#v ok=%v", session, ok)
	}
	if app.LSP == nil {
		t.Fatal("lsp manager should be wired")
	}
	if app.Goals == nil {
		t.Fatal("goal runtime should be wired")
	}
}

func TestAppGoalMethodsPublishEvents(t *testing.T) {
	app, err := New(context.Background(), Config{
		Workspace:   filepath.Join(t.TempDir(), "workspace"),
		DataDir:     t.TempDir(),
		Provider:    "mock",
		AutoApprove: true,
	})
	if err != nil {
		t.Fatalf("new app: %v", err)
	}
	defer app.Close(context.Background())

	events := app.Bus.Subscribe(context.Background(), eventbus.Filter{SessionID: "s1"})
	goal, err := app.SetGoal(context.Background(), goals.SetRequest{SessionID: "s1", Objective: "ship it"})
	if err != nil {
		t.Fatalf("set goal: %v", err)
	}
	if goal.Status != goals.StatusActive {
		t.Fatalf("goal = %#v", goal)
	}
	ev := <-events
	if ev.Kind != eventbus.KindGoalUpdated {
		t.Fatalf("event = %#v", ev)
	}
	var payload struct {
		SessionID       string `json:"session_id"`
		GoalID          string `json:"goal_id"`
		Status          string `json:"status"`
		TokensUsed      int64  `json:"tokens_used"`
		TimeUsedSeconds int64  `json:"time_used_seconds"`
	}
	if err := ev.DecodePayload(&payload); err != nil {
		t.Fatalf("decode payload: %v", err)
	}
	if payload.SessionID != "s1" || payload.GoalID == "" || payload.Status != "active" {
		t.Fatalf("payload = %#v", payload)
	}
	if _, err := app.SetGoal(context.Background(), goals.SetRequest{SessionID: "s1", Objective: "replace"}); !errors.Is(err, goals.ErrActiveGoalExists) {
		t.Fatalf("replace err = %v", err)
	}
	if err := app.ClearGoal(context.Background(), "s1"); err != nil {
		t.Fatalf("clear goal: %v", err)
	}
	ev = <-events
	if ev.Kind != eventbus.KindGoalCleared {
		t.Fatalf("clear event = %#v", ev)
	}
}

func TestAppWiresLSPManager(t *testing.T) {
	app, err := New(context.Background(), Config{
		Workspace:   filepath.Join(t.TempDir(), "workspace"),
		DataDir:     t.TempDir(),
		Provider:    "mock",
		AutoApprove: true,
	})
	if err != nil {
		t.Fatalf("new app: %v", err)
	}
	defer app.Close(context.Background())
	if app.LSP == nil {
		t.Fatal("lsp manager should be wired")
	}
}

func TestNewAppLoadsRepoContextMessages(t *testing.T) {
	workspace := filepath.Join(t.TempDir(), "workspace")
	if err := os.MkdirAll(workspace, 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if err := os.WriteFile(filepath.Join(workspace, "AGENTS.md"), []byte("repo rules"), 0o644); err != nil {
		t.Fatalf("write agents: %v", err)
	}
	if err := os.WriteFile(filepath.Join(workspace, ".gode.toml"), []byte(`[agent]
extra_context = "inline repo context"
`), 0o644); err != nil {
		t.Fatalf("write config: %v", err)
	}

	app, err := New(context.Background(), Config{
		Workspace:   workspace,
		DataDir:     t.TempDir(),
		Provider:    "mock",
		AutoApprove: true,
	})
	if err != nil {
		t.Fatalf("new app: %v", err)
	}
	defer app.Close(context.Background())

	joined := ""
	for _, msg := range app.contextMessages {
		joined += msg.Content + "\n"
	}
	for _, want := range []string{"repo rules", "AGENTS.md", "inline repo context"} {
		if !strings.Contains(joined, want) {
			t.Fatalf("context messages missing %q:\n%s", want, joined)
		}
	}
}

func TestNewAppDiscoversProjectSkills(t *testing.T) {
	workspace := filepath.Join(t.TempDir(), "workspace")
	skillDir := filepath.Join(workspace, ".agents", "skills", "go-tests")
	if err := os.MkdirAll(skillDir, 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if err := os.WriteFile(filepath.Join(skillDir, "SKILL.md"), []byte(`---
name: go-tests
description: Run tests
---
Run Go tests.
`), 0o644); err != nil {
		t.Fatalf("write skill: %v", err)
	}
	app, err := New(context.Background(), Config{
		Workspace:   workspace,
		DataDir:     t.TempDir(),
		Provider:    "mock",
		AutoApprove: true,
	})
	if err != nil {
		t.Fatalf("new app: %v", err)
	}
	defer app.Close(context.Background())

	if len(app.skills) != 1 || app.skills[0].Name != "go-tests" {
		t.Fatalf("skills = %#v", app.skills)
	}
}

func TestNewAppLoadsProjectCommands(t *testing.T) {
	workspace := filepath.Join(t.TempDir(), "workspace")
	commandPath := filepath.Join(workspace, ".gode", "commands", "test.md")
	if err := os.MkdirAll(filepath.Dir(commandPath), 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if err := os.WriteFile(commandPath, []byte("Run tests"), 0o644); err != nil {
		t.Fatalf("write command: %v", err)
	}
	app, err := New(context.Background(), Config{
		Workspace:   workspace,
		DataDir:     t.TempDir(),
		Provider:    "mock",
		AutoApprove: true,
	})
	if err != nil {
		t.Fatalf("new app: %v", err)
	}
	defer app.Close(context.Background())

	if len(app.commands) != 1 || app.commands[0].ID != "project:test" {
		t.Fatalf("commands = %#v", app.commands)
	}
}

func TestCompactSessionPersistsRawCompactionItems(t *testing.T) {
	dataDir := t.TempDir()
	sessionStore, err := session.Open(dataDir)
	if err != nil {
		t.Fatalf("session store: %v", err)
	}
	if _, err := sessionStore.Ensure(context.Background(), session.Session{ID: "s1", Title: "hello"}); err != nil {
		t.Fatalf("ensure session: %v", err)
	}
	messageStore := messagestore.Open(dataDir)
	if _, err := messageStore.Append(context.Background(), messagestore.Message{SessionID: "s1", Role: messagestore.RoleUser, Text: "hello"}); err != nil {
		t.Fatalf("append message: %v", err)
	}
	journalStore, err := journal.Open(filepath.Join(dataDir, "events.jsonl"))
	if err != nil {
		t.Fatalf("journal: %v", err)
	}
	defer journalStore.Close()
	compactor := &fakeCompactor{
		output: []json.RawMessage{
			json.RawMessage(`{"type":"message","role":"assistant","content":[{"type":"output_text","text":"summary"}]}`),
			json.RawMessage(`{"type":"compaction","encrypted_content":"opaque","id":"cmp_123"}`),
		},
	}
	app := &App{
		Config:   Config{Model: "gpt-5.5"},
		Bus:      eventbus.New(),
		Journal:  journalStore,
		Sessions: sessionStore,
		Messages: messageStore,
		provider: compactor,
	}
	defer app.Bus.Close()

	result, err := app.CompactSession(context.Background(), "s1")
	if err != nil {
		t.Fatalf("compact: %v", err)
	}
	if result.ResponseID != "cmp_resp" || result.OutputItems != 2 {
		t.Fatalf("result = %#v", result)
	}
	if len(compactor.request.Messages) != 1 || compactor.request.Messages[0].Content != "hello" {
		t.Fatalf("compact request messages = %#v", compactor.request.Messages)
	}
	messages, err := messageStore.ListBySession(context.Background(), "s1")
	if err != nil {
		t.Fatalf("list messages: %v", err)
	}
	if len(messages) != 3 {
		t.Fatalf("messages = %#v", messages)
	}
	last := messages[len(messages)-1]
	if last.Role != messagestore.RoleCompaction || last.SourceKind != "compacted" || !strings.Contains(string(last.RawJSON), `"encrypted_content":"opaque"`) {
		t.Fatalf("last message = %#v", last)
	}
	next := providerMessagesFromStored(messages)
	if len(next) != 2 || len(next[0].RawJSON) == 0 || len(next[1].RawJSON) == 0 {
		t.Fatalf("provider messages should preserve raw compaction item: %#v", next)
	}
	events, err := journalStore.Replay(context.Background(), journal.ReplayFilter{SessionID: "s1"})
	if err != nil {
		t.Fatalf("replay: %v", err)
	}
	kinds := map[eventbus.Kind]bool{}
	for _, ev := range events {
		kinds[ev.Kind] = true
	}
	if !kinds[eventbus.KindContextCompactionStarted] || !kinds[eventbus.KindContextCompactionCompleted] {
		t.Fatalf("missing compaction events: %#v", kinds)
	}
}

func TestCompactSessionFailureLeavesMessagesUntouched(t *testing.T) {
	dataDir := t.TempDir()
	sessionStore, err := session.Open(dataDir)
	if err != nil {
		t.Fatalf("session store: %v", err)
	}
	if _, err := sessionStore.Ensure(context.Background(), session.Session{ID: "s1", Title: "hello"}); err != nil {
		t.Fatalf("ensure session: %v", err)
	}
	messageStore := messagestore.Open(dataDir)
	if _, err := messageStore.Append(context.Background(), messagestore.Message{SessionID: "s1", Role: messagestore.RoleUser, Text: "hello"}); err != nil {
		t.Fatalf("append message: %v", err)
	}
	journalStore, err := journal.Open(filepath.Join(dataDir, "events.jsonl"))
	if err != nil {
		t.Fatalf("journal: %v", err)
	}
	defer journalStore.Close()
	app := &App{
		Config:   Config{Model: "gpt-5.5"},
		Bus:      eventbus.New(),
		Journal:  journalStore,
		Sessions: sessionStore,
		Messages: messageStore,
		provider: &fakeCompactor{err: errors.New("compact failed")},
	}
	defer app.Bus.Close()

	if _, err := app.CompactSession(context.Background(), "s1"); err == nil {
		t.Fatal("expected compact error")
	}
	messages, err := messageStore.ListBySession(context.Background(), "s1")
	if err != nil {
		t.Fatalf("list messages: %v", err)
	}
	if len(messages) != 1 || messages[0].Text != "hello" {
		t.Fatalf("messages should be untouched, got %#v", messages)
	}
	events, err := journalStore.Replay(context.Background(), journal.ReplayFilter{SessionID: "s1", Kinds: []eventbus.Kind{eventbus.KindContextCompactionFailed}})
	if err != nil {
		t.Fatalf("replay: %v", err)
	}
	if len(events) != 1 {
		t.Fatalf("failed events = %d", len(events))
	}
}

type fakeCompactor struct {
	request provider.CompactRequest
	output  []json.RawMessage
	err     error
}

func (f *fakeCompactor) Name() string {
	return "openai"
}

func (f *fakeCompactor) Stream(context.Context, provider.Request) (<-chan provider.Event, <-chan error) {
	events := make(chan provider.Event)
	errs := make(chan error)
	close(events)
	close(errs)
	return events, errs
}

func (f *fakeCompactor) Compact(_ context.Context, req provider.CompactRequest) (provider.CompactResult, error) {
	f.request = req
	if f.err != nil {
		return provider.CompactResult{}, f.err
	}
	return provider.CompactResult{ID: "cmp_resp", Output: f.output}, nil
}
