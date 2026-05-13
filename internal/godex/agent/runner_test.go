package agent

import (
	"context"
	"errors"
	"path/filepath"
	"strings"
	"testing"

	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/godex/journal"
	"github.com/pandelisz/gode/internal/godex/provider"
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
