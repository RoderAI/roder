package agent

import (
	"context"
	"errors"
	"path/filepath"
	"strings"
	"testing"

	godecommands "github.com/pandelisz/gode/internal/godex/commands"
	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/godex/journal"
	messagestore "github.com/pandelisz/gode/internal/godex/message"
	"github.com/pandelisz/gode/internal/godex/provider"
	"github.com/pandelisz/gode/internal/godex/session"
	godeskills "github.com/pandelisz/gode/internal/godex/skills"
	"github.com/pandelisz/gode/internal/godex/tools"
)

func TestRunnerPublishesAndJournalsMockTurn(t *testing.T) {
	bus := eventbus.New(eventbus.WithSubscriberBuffer(16))
	defer bus.Close()

	store, err := journal.Open(filepath.Join(t.TempDir(), "events.jsonl"))
	if err != nil {
		t.Fatalf("journal: %v", err)
	}
	defer store.Close()

	reg := tools.NewRegistry(tools.WithEventBus(bus), tools.WithAutoApprove(true))
	reg.Register(tools.Tool{
		Name:        "echo",
		Description: "echo",
		ReadOnly:    true,
		Run: func(context.Context, tools.Call) (tools.Result, error) {
			return tools.Result{Text: "echoed"}, nil
		},
	})

	runner := NewRunner(Config{
		Bus:      bus,
		Journal:  store,
		Tools:    reg,
		Provider: provider.NewMock("hello from mock", []provider.ToolRequest{{ID: "tc1", Name: "echo"}}),
	})

	result, err := runner.Run(context.Background(), RunRequest{
		SessionID: "s1",
		Prompt:    "hello",
	})
	if err != nil {
		t.Fatalf("run: %v", err)
	}
	if result.FinalText != "hello from mock" {
		t.Fatalf("final = %q", result.FinalText)
	}

	events, err := store.Replay(context.Background(), journal.ReplayFilter{SessionID: "s1"})
	if err != nil {
		t.Fatalf("replay: %v", err)
	}
	kinds := map[eventbus.Kind]bool{}
	for _, ev := range events {
		kinds[ev.Kind] = true
	}
	for _, want := range []eventbus.Kind{
		eventbus.KindUserPromptSubmitted,
		eventbus.KindRunStarted,
		eventbus.KindToolRequested,
		eventbus.KindToolCompleted,
		eventbus.KindAssistantCompleted,
		eventbus.KindRunCompleted,
	} {
		if !kinds[want] {
			t.Fatalf("missing event kind %q in %#v", want, kinds)
		}
	}
}

func TestRunnerSendsGodeInstructions(t *testing.T) {
	capture := &captureProvider{finalText: "done"}
	runner := NewRunner(Config{
		Bus:      eventbus.New(eventbus.WithSubscriberBuffer(16)),
		Provider: capture,
	})
	defer runner.bus.Close()

	if _, err := runner.Run(context.Background(), RunRequest{Prompt: "hello"}); err != nil {
		t.Fatalf("run: %v", err)
	}
	if capture.request.Instructions == "" {
		t.Fatal("instructions should be sent to provider")
	}
	for _, want := range []string{"You are gode", "Go-native coding agent", "dirty git worktree"} {
		if !strings.Contains(capture.request.Instructions, want) {
			t.Fatalf("instructions missing %q:\n%s", want, capture.request.Instructions)
		}
	}
	if len(capture.request.Messages) != 1 || capture.request.Messages[0].Content != "hello" {
		t.Fatalf("messages = %#v", capture.request.Messages)
	}
}

func TestRunnerPrependsContextMessages(t *testing.T) {
	capture := &captureProvider{finalText: "done"}
	runner := NewRunner(Config{
		Bus:      eventbus.New(eventbus.WithSubscriberBuffer(16)),
		Provider: capture,
		ContextMessages: []provider.Message{{
			Role:    provider.RoleSystem,
			Content: "<repo-context-file path=\"AGENTS.md\">rules</repo-context-file>",
		}},
	})
	defer runner.bus.Close()

	if _, err := runner.Run(context.Background(), RunRequest{Prompt: "hello"}); err != nil {
		t.Fatalf("run: %v", err)
	}
	if len(capture.request.Messages) != 2 {
		t.Fatalf("messages = %#v", capture.request.Messages)
	}
	if capture.request.Messages[0].Role != provider.RoleSystem || !strings.Contains(capture.request.Messages[0].Content, "AGENTS.md") {
		t.Fatalf("context message = %#v", capture.request.Messages[0])
	}
	if capture.request.Messages[1].Role != provider.RoleUser || capture.request.Messages[1].Content != "hello" {
		t.Fatalf("user message = %#v", capture.request.Messages[1])
	}
}

func TestRunnerInjectsInvokedSkillsAndCleansPrompt(t *testing.T) {
	capture := &captureProvider{finalText: "done"}
	runner := NewRunner(Config{
		Bus:      eventbus.New(eventbus.WithSubscriberBuffer(16)),
		Provider: capture,
		Skills: []godeskills.Skill{{
			Name: "go-tests",
			Body: "Run Go tests before reporting completion.",
		}},
	})
	defer runner.bus.Close()

	if _, err := runner.Run(context.Background(), RunRequest{Prompt: "$go-tests please check this"}); err != nil {
		t.Fatalf("run: %v", err)
	}
	if len(capture.request.Messages) != 2 {
		t.Fatalf("messages = %#v", capture.request.Messages)
	}
	if capture.request.Messages[0].Role != provider.RoleSystem || !strings.Contains(capture.request.Messages[0].Content, `<skill name="go-tests">`) {
		t.Fatalf("skill message = %#v", capture.request.Messages[0])
	}
	if capture.request.Messages[1].Role != provider.RoleUser || capture.request.Messages[1].Content != "please check this" {
		t.Fatalf("user message = %#v", capture.request.Messages[1])
	}
}

func TestRunnerExpandsSlashCommandsBeforeProviderRequest(t *testing.T) {
	capture := &captureProvider{finalText: "done"}
	runner := NewRunner(Config{
		Bus:      eventbus.New(eventbus.WithSubscriberBuffer(16)),
		Provider: capture,
		Commands: []godecommands.Command{{
			ID:           "project:test",
			Scope:        "project",
			Prompt:       "Run $TARGET tests",
			Placeholders: []string{"TARGET"},
		}},
	})
	defer runner.bus.Close()

	if _, err := runner.Run(context.Background(), RunRequest{Prompt: "/test TARGET=api with coverage"}); err != nil {
		t.Fatalf("run: %v", err)
	}
	if len(capture.request.Messages) != 1 {
		t.Fatalf("messages = %#v", capture.request.Messages)
	}
	if capture.request.Messages[0].Content != "Run api tests\n\nwith coverage" {
		t.Fatalf("prompt = %q", capture.request.Messages[0].Content)
	}
}

func TestRunnerDoesNotTreatAbsolutePathAsSlashCommand(t *testing.T) {
	capture := &captureProvider{finalText: "done"}
	runner := NewRunner(Config{
		Bus:      eventbus.New(eventbus.WithSubscriberBuffer(16)),
		Provider: capture,
		Commands: []godecommands.Command{{
			ID:     "project:Users",
			Scope:  "project",
			Prompt: "wrong",
		}},
	})
	defer runner.bus.Close()

	prompt := "/Users/pz/file.go is a path"
	if _, err := runner.Run(context.Background(), RunRequest{Prompt: prompt}); err != nil {
		t.Fatalf("run: %v", err)
	}
	if len(capture.request.Messages) != 1 || capture.request.Messages[0].Content != prompt {
		t.Fatalf("messages = %#v", capture.request.Messages)
	}
}

func TestRunnerCarriesFunctionCallBeforeToolOutput(t *testing.T) {
	reg := tools.NewRegistry(tools.WithAutoApprove(true))
	reg.Register(tools.Tool{
		Name:        "echo",
		Description: "echo",
		ReadOnly:    true,
		Run: func(context.Context, tools.Call) (tools.Result, error) {
			return tools.Result{Text: "echoed"}, nil
		},
	})
	script := &scriptedProvider{
		streams: [][]provider.Event{
			{
				{
					Kind: provider.EventToolCall,
					ToolRequest: &provider.ToolRequest{
						ID:        "call_abc",
						Name:      "echo",
						Input:     map[string]any{"text": "hello"},
						Arguments: `{"text":"hello"}`,
					},
				},
				{Kind: provider.EventCompleted},
			},
			{
				{Kind: provider.EventDelta, Text: "done"},
				{Kind: provider.EventCompleted, Text: "done"},
			},
		},
	}
	runner := NewRunner(Config{
		Bus:      eventbus.New(eventbus.WithSubscriberBuffer(16)),
		Tools:    reg,
		Provider: script,
	})
	defer runner.bus.Close()

	if _, err := runner.Run(context.Background(), RunRequest{Prompt: "hello"}); err != nil {
		t.Fatalf("run: %v", err)
	}
	if len(script.requests) != 2 {
		t.Fatalf("requests = %d, want 2", len(script.requests))
	}
	messages := script.requests[1].Messages
	if len(messages) != 3 {
		t.Fatalf("second request messages = %#v", messages)
	}
	if messages[1].Role != provider.RoleAssistant || messages[1].ToolCallID != "call_abc" || messages[1].ToolName != "echo" || messages[1].ToolArguments != `{"text":"hello"}` {
		t.Fatalf("assistant function call message = %#v", messages[1])
	}
	if messages[2].Role != provider.RoleTool || messages[2].ToolCallID != "call_abc" || !strings.Contains(messages[2].Content, "echoed") {
		t.Fatalf("tool output message = %#v", messages[2])
	}
}

func TestRunnerFeedsToolFailureBackToModel(t *testing.T) {
	reg := tools.NewRegistry(tools.WithAutoApprove(true))
	reg.Register(tools.Tool{
		Name:        "apply_patch",
		Description: "patch",
		ReadOnly:    false,
		Run: func(context.Context, tools.Call) (tools.Result, error) {
			return tools.Result{Text: "error: corrupt patch at line 4"}, errors.New("failed to apply patch: exit status 128")
		},
	})
	script := &scriptedProvider{
		streams: [][]provider.Event{
			{
				{
					Kind: provider.EventToolCall,
					ToolRequest: &provider.ToolRequest{
						ID:        "call_patch",
						Name:      "apply_patch",
						Input:     map[string]any{"patch": "bad"},
						Arguments: `{"patch":"bad"}`,
					},
				},
				{Kind: provider.EventCompleted},
			},
			{
				{Kind: provider.EventDelta, Text: "recovered"},
				{Kind: provider.EventCompleted, Text: "recovered"},
			},
		},
	}
	runner := NewRunner(Config{
		Bus:      eventbus.New(eventbus.WithSubscriberBuffer(16)),
		Tools:    reg,
		Provider: script,
	})
	defer runner.bus.Close()

	result, err := runner.Run(context.Background(), RunRequest{Prompt: "patch this"})
	if err != nil {
		t.Fatalf("run: %v", err)
	}
	if result.FinalText != "recovered" {
		t.Fatalf("final = %q", result.FinalText)
	}
	if len(script.requests) != 2 {
		t.Fatalf("requests = %d, want 2", len(script.requests))
	}
	messages := script.requests[1].Messages
	if len(messages) != 3 {
		t.Fatalf("second request messages = %#v", messages)
	}
	if messages[2].Role != provider.RoleTool || messages[2].ToolCallID != "call_patch" {
		t.Fatalf("tool output message = %#v", messages[2])
	}
	for _, want := range []string{"Tool apply_patch failed", "failed to apply patch: exit status 128", "error: corrupt patch at line 4"} {
		if !strings.Contains(messages[2].Content, want) {
			t.Fatalf("tool output missing %q:\n%s", want, messages[2].Content)
		}
	}
}

func TestRunnerToolLoopFailureIncludesDebugDetail(t *testing.T) {
	reg := tools.NewRegistry(tools.WithAutoApprove(true))
	reg.Register(tools.Tool{
		Name:        "echo",
		Description: "echo",
		ReadOnly:    true,
		Run: func(context.Context, tools.Call) (tools.Result, error) {
			return tools.Result{Text: "again"}, nil
		},
	})
	dataDir := t.TempDir()
	store, err := journal.Open(filepath.Join(dataDir, "events.jsonl"))
	if err != nil {
		t.Fatalf("journal: %v", err)
	}
	defer store.Close()
	messageStore := messagestore.Open(dataDir)
	script := &scriptedProvider{
		streams: [][]provider.Event{
			{
				{Kind: provider.EventToolCall, ToolRequest: &provider.ToolRequest{ID: "call_1", Name: "echo", Arguments: `{}`}},
				{Kind: provider.EventCompleted},
			},
			{
				{Kind: provider.EventToolCall, ToolRequest: &provider.ToolRequest{ID: "call_2", Name: "echo", Arguments: `{}`}},
				{Kind: provider.EventCompleted},
			},
		},
	}
	runner := NewRunner(Config{
		Bus:      eventbus.New(eventbus.WithSubscriberBuffer(16)),
		Journal:  store,
		Messages: messageStore,
		Tools:    reg,
		Provider: script,
	})
	defer runner.bus.Close()

	_, err = runner.Run(context.Background(), RunRequest{SessionID: "s-loop", RunID: "r-loop", Prompt: "loop", MaxTurns: 2})
	if err == nil {
		t.Fatal("expected tool loop error")
	}
	for _, want := range []string{
		"agent stopped without final text after tool loop",
		"session_id: s-loop",
		"run_id: r-loop",
		"max_turns: 2",
		"tool_turns: 2",
		"tool_calls: 2",
		"last_tool: echo",
		"last_tool_call_id: call_2",
		"event_journal: " + filepath.Join(dataDir, "events.jsonl"),
		"message_log: " + filepath.Join(dataDir, "sessions", "s-loop", "messages.jsonl"),
	} {
		if !strings.Contains(err.Error(), want) {
			t.Fatalf("error missing %q:\n%s", want, err)
		}
	}
	events, err := store.Replay(context.Background(), journal.ReplayFilter{SessionID: "s-loop", RunID: "r-loop", Kinds: []eventbus.Kind{eventbus.KindRunFailed}})
	if err != nil {
		t.Fatalf("replay: %v", err)
	}
	if len(events) != 1 {
		t.Fatalf("run failed events = %d", len(events))
	}
	var payload struct {
		Error  string `json:"error"`
		Detail string `json:"detail"`
	}
	if err := events[0].DecodePayload(&payload); err != nil {
		t.Fatalf("decode payload: %v", err)
	}
	if payload.Error != "agent stopped without final text after tool loop" {
		t.Fatalf("summary = %q", payload.Error)
	}
	if !strings.Contains(payload.Detail, "last_tool_call_id: call_2") {
		t.Fatalf("detail missing last call:\n%s", payload.Detail)
	}
}

func TestRunnerResumeLoadsPriorMessages(t *testing.T) {
	messageStore := messagestore.Open(t.TempDir())
	if _, err := messageStore.Append(context.Background(), messagestore.Message{SessionID: "s1", RunID: "old", Role: messagestore.RoleUser, Text: "previous prompt"}); err != nil {
		t.Fatalf("append prior user: %v", err)
	}
	if _, err := messageStore.Append(context.Background(), messagestore.Message{SessionID: "s1", RunID: "old", Role: messagestore.RoleAssistant, Text: "previous answer"}); err != nil {
		t.Fatalf("append prior assistant: %v", err)
	}
	script := &scriptedProvider{streams: [][]provider.Event{{
		{Kind: provider.EventDelta, Text: "done"},
		{Kind: provider.EventCompleted, Text: "done"},
	}}}
	runner := NewRunner(Config{
		Bus:      eventbus.New(eventbus.WithSubscriberBuffer(16)),
		Messages: messageStore,
		Provider: script,
	})
	defer runner.bus.Close()

	if _, err := runner.Run(context.Background(), RunRequest{SessionID: "s1", Prompt: "next prompt", Resume: true}); err != nil {
		t.Fatalf("run: %v", err)
	}
	got := script.requests[0].Messages
	if len(got) != 3 {
		t.Fatalf("messages = %#v", got)
	}
	if got[0].Role != provider.RoleUser || got[0].Content != "previous prompt" {
		t.Fatalf("prior user = %#v", got[0])
	}
	if got[1].Role != provider.RoleAssistant || got[1].Content != "previous answer" {
		t.Fatalf("prior assistant = %#v", got[1])
	}
	if got[2].Role != provider.RoleUser || got[2].Content != "next prompt" {
		t.Fatalf("new prompt = %#v", got[2])
	}
}

func TestRunnerWithoutResumeIgnoresPriorMessages(t *testing.T) {
	messageStore := messagestore.Open(t.TempDir())
	if _, err := messageStore.Append(context.Background(), messagestore.Message{SessionID: "s1", Role: messagestore.RoleUser, Text: "previous prompt"}); err != nil {
		t.Fatalf("append prior: %v", err)
	}
	script := &scriptedProvider{streams: [][]provider.Event{{
		{Kind: provider.EventDelta, Text: "done"},
		{Kind: provider.EventCompleted, Text: "done"},
	}}}
	runner := NewRunner(Config{
		Bus:      eventbus.New(eventbus.WithSubscriberBuffer(16)),
		Messages: messageStore,
		Provider: script,
	})
	defer runner.bus.Close()

	if _, err := runner.Run(context.Background(), RunRequest{SessionID: "s1", Prompt: "fresh prompt"}); err != nil {
		t.Fatalf("run: %v", err)
	}
	got := script.requests[0].Messages
	if len(got) != 1 || got[0].Content != "fresh prompt" {
		t.Fatalf("messages = %#v", got)
	}
}

func TestRunnerPersistsSessionAndMessages(t *testing.T) {
	dataDir := t.TempDir()
	sessionStore := openSessionStore(t, dataDir)
	messageStore := messagestore.Open(dataDir)
	runner := NewRunner(Config{
		Bus:      eventbus.New(eventbus.WithSubscriberBuffer(16)),
		Sessions: sessionStore,
		Messages: messageStore,
		Provider: &scriptedProvider{streams: [][]provider.Event{{
			{Kind: provider.EventDelta, Text: "done"},
			{Kind: provider.EventCompleted, Text: "done"},
		}}},
	})
	defer runner.bus.Close()

	if _, err := runner.Run(context.Background(), RunRequest{SessionID: "s-persist", Prompt: "hello"}); err != nil {
		t.Fatalf("run: %v", err)
	}
	messages, err := messageStore.ListBySession(context.Background(), "s-persist")
	if err != nil {
		t.Fatalf("list messages: %v", err)
	}
	if len(messages) != 2 || messages[0].Text != "hello" || messages[1].Text != "done" {
		t.Fatalf("messages = %#v", messages)
	}
	stored, ok, err := sessionStore.Get(context.Background(), "s-persist")
	if err != nil {
		t.Fatalf("get session: %v", err)
	}
	if !ok || stored.Title != "hello" || stored.MessageCount != 2 {
		t.Fatalf("session = %#v ok=%v", stored, ok)
	}
}

func TestRunnerPublishesReasoningSummaryEvents(t *testing.T) {
	bus := eventbus.New(eventbus.WithSubscriberBuffer(16))
	defer bus.Close()

	store, err := journal.Open(filepath.Join(t.TempDir(), "events.jsonl"))
	if err != nil {
		t.Fatalf("journal: %v", err)
	}
	defer store.Close()

	runner := NewRunner(Config{
		Bus:     bus,
		Journal: store,
		Provider: &scriptedProvider{streams: [][]provider.Event{{
			{Kind: provider.EventReasoningSummaryDelta, Text: "Checking files"},
			{Kind: provider.EventReasoningSummaryDone, Text: "Checking files before editing."},
			{Kind: provider.EventDelta, Text: "done"},
			{Kind: provider.EventCompleted, Text: "done"},
		}}},
	})

	if _, err := runner.Run(context.Background(), RunRequest{SessionID: "s-reasoning", Prompt: "hello"}); err != nil {
		t.Fatalf("run: %v", err)
	}

	events, err := store.Replay(context.Background(), journal.ReplayFilter{SessionID: "s-reasoning"})
	if err != nil {
		t.Fatalf("replay: %v", err)
	}
	kinds := map[eventbus.Kind]string{}
	for _, ev := range events {
		var payload struct {
			Text string `json:"text"`
		}
		_ = ev.DecodePayload(&payload)
		kinds[ev.Kind] = payload.Text
	}
	if kinds[eventbus.KindReasoningSummaryDelta] != "Checking files" {
		t.Fatalf("reasoning delta = %q", kinds[eventbus.KindReasoningSummaryDelta])
	}
	if kinds[eventbus.KindReasoningSummaryCompleted] != "Checking files before editing." {
		t.Fatalf("reasoning completed = %q", kinds[eventbus.KindReasoningSummaryCompleted])
	}
}

func openSessionStore(t *testing.T, dataDir string) *session.Store {
	t.Helper()
	store, err := session.Open(dataDir)
	if err != nil {
		t.Fatalf("session store: %v", err)
	}
	return store
}

type captureProvider struct {
	request   provider.Request
	finalText string
}

func (p *captureProvider) Name() string {
	return "capture"
}

func (p *captureProvider) Stream(_ context.Context, req provider.Request) (<-chan provider.Event, <-chan error) {
	p.request = req
	events := make(chan provider.Event, 2)
	errs := make(chan error)
	events <- provider.Event{Kind: provider.EventDelta, Text: p.finalText}
	events <- provider.Event{Kind: provider.EventCompleted, Text: p.finalText}
	close(events)
	close(errs)
	return events, errs
}

type scriptedProvider struct {
	requests []provider.Request
	streams  [][]provider.Event
}

func (p *scriptedProvider) Name() string {
	return "scripted"
}

func (p *scriptedProvider) Stream(_ context.Context, req provider.Request) (<-chan provider.Event, <-chan error) {
	p.requests = append(p.requests, req)
	events := make(chan provider.Event, 8)
	errs := make(chan error)
	index := len(p.requests) - 1
	if index < len(p.streams) {
		for _, ev := range p.streams[index] {
			events <- ev
		}
	}
	close(events)
	close(errs)
	return events, errs
}
