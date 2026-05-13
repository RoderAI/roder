package agent

import (
	"context"
	"path/filepath"
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
