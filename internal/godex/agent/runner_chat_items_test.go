package agent

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"testing"

	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/godex/provider"
	"github.com/pandelisz/gode/internal/godex/session"
	"github.com/pandelisz/gode/internal/godex/tools"
)

func TestRunnerPersistsChatCompletionAssistantTextAsCanonicalItem(t *testing.T) {
	dataDir := t.TempDir()
	itemStore := openItemStore(t, dataDir)
	script := &scriptedProvider{streams: [][]provider.Event{{
		{
			Kind: provider.EventCompleted,
			Text: "done",
			Items: []provider.Item{{
				ID:   "chat_msg_1",
				Kind: provider.ItemMessage,
				Role: "assistant",
				Text: "done",
			}},
		},
	}}}
	runner := NewRunner(Config{
		Bus:      eventbus.New(eventbus.WithSubscriberBuffer(16)),
		Items:    itemStore,
		Provider: script,
	})
	defer runner.bus.Close()

	if _, err := runner.Run(context.Background(), RunRequest{SessionID: "s-chat-text", RunID: "r-chat-text", Prompt: "hello"}); err != nil {
		t.Fatalf("run: %v", err)
	}
	items, err := itemStore.ListBySession(context.Background(), "s-chat-text")
	if err != nil {
		t.Fatalf("items: %v", err)
	}
	if len(items) != 2 {
		t.Fatalf("items = %#v", items)
	}
	if items[1].Kind != session.ItemMessage || items[1].Role != "assistant" || items[1].Text != "done" || items[1].ID != "chat_msg_1" {
		t.Fatalf("assistant item = %#v", items[1])
	}
}

func TestRunnerPersistsChatToolLoopAsCanonicalItemsAndReplaysThem(t *testing.T) {
	dataDir := t.TempDir()
	itemStore := openItemStore(t, dataDir)
	registry := tools.NewRegistry()
	registry.Register(tools.Tool{
		Name: "echo",
		Run: func(context.Context, tools.Call) (tools.Result, error) {
			return tools.Result{Text: "tool output"}, nil
		},
	})
	script := &scriptedProvider{streams: [][]provider.Event{
		{
			{
				Kind: provider.EventToolCall,
				ToolRequest: &provider.ToolRequest{
					ID:        "call_1",
					Name:      "echo",
					Input:     map[string]any{"value": "x"},
					Arguments: `{"value":"x"}`,
				},
				Items: []provider.Item{{
					ID:         "call_1",
					Kind:       provider.ItemFunctionCall,
					ToolName:   "echo",
					ToolCallID: "call_1",
					Text:       `{"value":"x"}`,
				}},
			},
			{
				Kind: provider.EventCompleted,
				Items: []provider.Item{{
					ID:         "call_1",
					Kind:       provider.ItemFunctionCall,
					ToolName:   "echo",
					ToolCallID: "call_1",
					Text:       `{"value":"x"}`,
				}},
			},
		},
		{
			{
				Kind: provider.EventCompleted,
				Text: "done",
				Items: []provider.Item{{
					ID:   "msg_done",
					Kind: provider.ItemMessage,
					Role: "assistant",
					Text: "done",
				}},
			},
		},
	}}
	runner := NewRunner(Config{
		Bus:      eventbus.New(eventbus.WithSubscriberBuffer(32)),
		Items:    itemStore,
		Tools:    registry,
		Provider: script,
	})
	defer runner.bus.Close()

	if _, err := runner.Run(context.Background(), RunRequest{SessionID: "s-chat-tool", RunID: "r-chat-tool", Prompt: "use tool"}); err != nil {
		t.Fatalf("run: %v", err)
	}
	if len(script.requests) != 2 {
		t.Fatalf("requests = %d", len(script.requests))
	}
	replayed := script.requests[1].InputItems
	callIndex, outIndex := findItemIndexes(replayed, provider.ItemFunctionCall, provider.ItemFunctionOut)
	if callIndex == -1 || outIndex == -1 || callIndex > outIndex {
		t.Fatalf("replayed input items = %#v", replayed)
	}
	if replayed[callIndex].ToolName != "echo" || replayed[callIndex].ToolCallID != "call_1" || replayed[callIndex].Text != `{"value":"x"}` {
		t.Fatalf("replayed tool call = %#v", replayed[callIndex])
	}
	if replayed[outIndex].ToolCallID != "call_1" || !strings.Contains(replayed[outIndex].Text, "tool output") {
		t.Fatalf("replayed tool output = %#v", replayed[outIndex])
	}

	stored, err := itemStore.ListBySession(context.Background(), "s-chat-tool")
	if err != nil {
		t.Fatalf("stored items: %v", err)
	}
	if got := countSessionKind(stored, session.ItemFunctionCall); got != 1 {
		t.Fatalf("function call count = %d, items = %#v", got, stored)
	}
	if got := countSessionKind(stored, session.ItemFunctionOut); got != 1 {
		t.Fatalf("function output count = %d, items = %#v", got, stored)
	}
}

func TestRunnerResumeSendsFullCanonicalContextWithoutPreviousResponseID(t *testing.T) {
	dataDir := t.TempDir()
	itemStore := openItemStore(t, dataDir)
	if _, err := itemStore.AppendMany(context.Background(), []session.Item{
		{SessionID: "s-chat-resume", TurnID: "old", Kind: session.ItemMessage, Role: "user", Text: "previous"},
		{SessionID: "s-chat-resume", TurnID: "old", Kind: session.ItemMessage, Role: "assistant", Text: "answer"},
	}); err != nil {
		t.Fatalf("append prior items: %v", err)
	}
	script := &scriptedProvider{streams: [][]provider.Event{{
		{Kind: provider.EventCompleted, Text: "done"},
	}}}
	runner := NewRunner(Config{
		Bus:      eventbus.New(eventbus.WithSubscriberBuffer(16)),
		Items:    itemStore,
		Provider: script,
	})
	defer runner.bus.Close()

	if _, err := runner.Run(context.Background(), RunRequest{SessionID: "s-chat-resume", RunID: "new", Prompt: "continue", Resume: true}); err != nil {
		t.Fatalf("run: %v", err)
	}
	if script.requests[0].PreviousResponseID != "" {
		t.Fatalf("previous response id should not be replayed for local canonical context: %#v", script.requests[0])
	}
	if len(script.requests[0].InputItems) != 3 {
		t.Fatalf("input items = %#v", script.requests[0].InputItems)
	}
}

func TestRunnerReplaysNeutralCompactionItemsForChatRequests(t *testing.T) {
	dataDir := t.TempDir()
	itemStore := openItemStore(t, dataDir)
	if _, err := itemStore.Append(context.Background(), session.Item{
		SessionID: "s-chat-compact",
		TurnID:    "compact",
		Kind:      session.ItemCompaction,
		Text:      "Earlier context summary.",
	}); err != nil {
		t.Fatalf("append compaction: %v", err)
	}
	script := &scriptedProvider{streams: [][]provider.Event{{
		{Kind: provider.EventCompleted, Text: "done"},
	}}}
	runner := NewRunner(Config{
		Bus:      eventbus.New(eventbus.WithSubscriberBuffer(16)),
		Items:    itemStore,
		Provider: script,
	})
	defer runner.bus.Close()

	if _, err := runner.Run(context.Background(), RunRequest{SessionID: "s-chat-compact", RunID: "next", Prompt: "continue", Resume: true}); err != nil {
		t.Fatalf("run: %v", err)
	}
	items := script.requests[0].InputItems
	if len(items) != 2 || items[0].Kind != provider.ItemCompaction || items[0].Text != "Earlier context summary." {
		t.Fatalf("input items = %#v", items)
	}
	input, err := provider.ChatInputFromResponsesItems(items, nil)
	if err != nil {
		t.Fatalf("chat input: %v", err)
	}
	if len(input.Messages) != 2 || input.Messages[0].Content != "Earlier context summary." || input.Messages[1].Content != "continue" {
		t.Fatalf("chat messages = %#v", input.Messages)
	}
}

func TestChatCompletionsRejectProviderSpecificCompactionItems(t *testing.T) {
	_, err := provider.ChatInputFromResponsesItems([]provider.Item{{
		ID:      "cmp_raw",
		Kind:    provider.ItemCompaction,
		RawJSON: json.RawMessage(`{"type":"compaction","encrypted_content":"opaque"}`),
	}}, nil)
	if err == nil {
		t.Fatal("expected nonportable compaction error")
	}
	var portable provider.NonPortableItemError
	if !errors.As(err, &portable) {
		t.Fatalf("error = %T %v", err, err)
	}
	if !strings.Contains(err.Error(), "nonportable") || !strings.Contains(err.Error(), "provider-neutral compaction text") {
		t.Fatalf("error = %v", err)
	}
}

func findItemIndexes(items []provider.Item, first provider.ItemKind, second provider.ItemKind) (int, int) {
	firstIndex, secondIndex := -1, -1
	for i, item := range items {
		if firstIndex == -1 && item.Kind == first {
			firstIndex = i
		}
		if secondIndex == -1 && item.Kind == second {
			secondIndex = i
		}
	}
	return firstIndex, secondIndex
}

func countSessionKind(items []session.Item, kind session.ItemKind) int {
	count := 0
	for _, item := range items {
		if item.Kind == kind {
			count++
		}
	}
	return count
}
