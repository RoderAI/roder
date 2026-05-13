package agent

import (
	"context"
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
