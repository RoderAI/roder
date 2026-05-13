package agent

import (
	"context"
	"testing"

	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/godex/provider"
	"github.com/pandelisz/gode/internal/godex/tools"
)

func TestRunnerAppliesSteerBeforeNextProviderTurn(t *testing.T) {
	reg := tools.NewRegistry(tools.WithAutoApprove(true))
	script := &scriptedProvider{
		streams: [][]provider.Event{
			{
				{Kind: provider.EventToolCall, ToolRequest: &provider.ToolRequest{ID: "call_1", Name: "pause", Arguments: `{}`}},
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
	reg.Register(tools.Tool{
		Name:        "pause",
		Description: "pause",
		ReadOnly:    true,
		Run: func(ctx context.Context, call tools.Call) (tools.Result, error) {
			if _, err := runner.Steer(ctx, SteerRequest{SessionID: call.SessionID, Prompt: "use the new constraint"}); err != nil {
				t.Fatalf("steer: %v", err)
			}
			return tools.Result{Text: "tool done"}, nil
		},
	})

	result, err := runner.Run(context.Background(), RunRequest{SessionID: "s-steer", RunID: "r-steer", Prompt: "start"})
	if err != nil {
		t.Fatalf("run: %v", err)
	}
	if result.FinalText != "done" {
		t.Fatalf("final text = %q", result.FinalText)
	}
	if len(script.requests) != 2 {
		t.Fatalf("requests = %d, want 2", len(script.requests))
	}
	messages := script.requests[1].Messages
	if len(messages) < 4 {
		t.Fatalf("second request messages = %#v", messages)
	}
	if got := messages[len(messages)-1]; got.Role != provider.RoleUser || got.Content != "use the new constraint" {
		t.Fatalf("last message = %#v, want steer user message", got)
	}
}

func TestRunnerSteerRejectsMissingOrMismatchedActiveRun(t *testing.T) {
	runner := NewRunner(Config{Provider: &scriptedProvider{}})
	if _, err := runner.Steer(context.Background(), SteerRequest{SessionID: "missing", Prompt: "hi"}); err != ErrNoActiveRun {
		t.Fatalf("missing err = %v", err)
	}
	active := runner.registerActiveRun(RunRequest{SessionID: "s1", RunID: "r1"})
	defer runner.unregisterActiveRun(active)
	if _, err := runner.Steer(context.Background(), SteerRequest{SessionID: "s1", ExpectedRunID: "r2", Prompt: "hi"}); err == nil {
		t.Fatal("expected mismatch error")
	}
}
