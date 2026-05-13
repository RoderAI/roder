package agent

import (
	"context"
	"strings"
	"testing"

	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/godex/provider"
	"github.com/pandelisz/gode/internal/godex/tools"
)

func TestRunnerFinalizesAfterToolTurnBudgetWithoutTools(t *testing.T) {
	reg := tools.NewRegistry(tools.WithAutoApprove(true))
	reg.Register(tools.Tool{
		Name:        "echo",
		Description: "echo",
		ReadOnly:    true,
		Run: func(context.Context, tools.Call) (tools.Result, error) {
			return tools.Result{Text: "tool output"}, nil
		},
	})
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
			{
				{Kind: provider.EventDelta, Text: "final answer"},
				{Kind: provider.EventCompleted, Text: "final answer"},
			},
		},
	}
	runner := NewRunner(Config{
		Bus:      eventbus.New(eventbus.WithSubscriberBuffer(16)),
		Tools:    reg,
		Provider: script,
	})
	defer runner.bus.Close()

	result, err := runner.Run(context.Background(), RunRequest{Prompt: "loop", MaxTurns: 2})
	if err != nil {
		t.Fatalf("run: %v", err)
	}
	if result.FinalText != "final answer" {
		t.Fatalf("final = %q", result.FinalText)
	}
	if len(script.requests) != 3 {
		t.Fatalf("requests = %d, want 3", len(script.requests))
	}
	finalReq := script.requests[2]
	if len(finalReq.Tools) != 0 {
		t.Fatalf("finalization request should not expose tools: %#v", finalReq.Tools)
	}
	if len(finalReq.Messages) == 0 || !strings.Contains(finalReq.Messages[len(finalReq.Messages)-1].Content, "Tool-call budget reached") {
		t.Fatalf("finalization prompt missing:\n%#v", finalReq.Messages)
	}
}
