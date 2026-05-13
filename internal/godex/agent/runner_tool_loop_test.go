package agent

import (
	"context"
	"strings"
	"testing"

	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/godex/provider"
	"github.com/pandelisz/gode/internal/godex/session"
	"github.com/pandelisz/gode/internal/godex/tools"
)

func TestRunnerToolLoopContinuesAfterTextBeforeToolTurn(t *testing.T) {
	reg := tools.NewRegistry(tools.WithAutoApprove(true))
	reg.Register(tools.Tool{
		Name:        "read_file",
		Description: "read",
		ReadOnly:    true,
		Run: func(context.Context, tools.Call) (tools.Result, error) {
			return tools.Result{Text: "README contents"}, nil
		},
	})
	itemStore, err := session.OpenItemStore(t.TempDir())
	if err != nil {
		t.Fatalf("item store: %v", err)
	}
	script := &scriptedProvider{
		streams: [][]provider.Event{
			{
				{Kind: provider.EventDelta, Text: "I'll inspect that."},
				{Kind: provider.EventToolCall, ToolRequest: &provider.ToolRequest{ID: "toolu_01", Name: "read_file", Arguments: `{"path":"README.md"}`}},
				{Kind: provider.EventCompleted, Text: "I'll inspect that."},
			},
			{
				{Kind: provider.EventDelta, Text: "final answer"},
				{Kind: provider.EventCompleted, Text: "final answer"},
			},
		},
	}
	runner := NewRunner(Config{
		Bus:      eventbus.New(eventbus.WithSubscriberBuffer(16)),
		Items:    itemStore,
		Tools:    reg,
		Provider: script,
	})
	defer runner.bus.Close()

	result, err := runner.Run(context.Background(), RunRequest{SessionID: "s-text-tool", RunID: "r-text-tool", Prompt: "inspect"})
	if err != nil {
		t.Fatalf("run: %v", err)
	}
	if result.FinalText != "final answer" {
		t.Fatalf("final = %q", result.FinalText)
	}
	if len(script.requests) != 2 {
		t.Fatalf("requests = %d, want 2", len(script.requests))
	}
	second := script.requests[1].Messages
	if len(second) != 3 {
		t.Fatalf("second request messages = %#v", second)
	}
	if second[1].Role != provider.RoleAssistant || second[1].ToolCallID != "toolu_01" || second[1].ToolName != "read_file" {
		t.Fatalf("assistant tool message = %#v", second[1])
	}
	if second[2].Role != provider.RoleTool || second[2].ToolCallID != "toolu_01" || !strings.Contains(second[2].Content, "README contents") {
		t.Fatalf("tool result message = %#v", second[2])
	}
	items, err := itemStore.ListBySession(context.Background(), "s-text-tool")
	if err != nil {
		t.Fatalf("list items: %v", err)
	}
	var sawPreface bool
	for _, item := range items {
		if item.Kind == session.ItemMessage && item.Role == "assistant" && item.Text == "I'll inspect that." {
			sawPreface = true
		}
	}
	if !sawPreface {
		t.Fatalf("assistant preface was not persisted as a canonical item: %#v", items)
	}
}

func TestRunnerContinuesToolTurnsUntilProviderStopsRequestingTools(t *testing.T) {
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

	result, err := runner.Run(context.Background(), RunRequest{Prompt: "loop"})
	if err != nil {
		t.Fatalf("run: %v", err)
	}
	if result.FinalText != "final answer" {
		t.Fatalf("final = %q", result.FinalText)
	}
	if len(script.requests) != 3 {
		t.Fatalf("requests = %d, want 3", len(script.requests))
	}
	for i, req := range script.requests {
		if len(req.Tools) == 0 {
			t.Fatalf("request %d should keep tools available until the provider stops asking: %#v", i, req.Tools)
		}
	}
}
