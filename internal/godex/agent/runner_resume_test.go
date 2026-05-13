package agent

import (
	"context"
	"encoding/json"
	"strings"
	"testing"

	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/godex/provider"
	"github.com/pandelisz/gode/internal/godex/session"
)

func TestRunnerResumeLoadsPriorItemsFromDisk(t *testing.T) {
	dataDir := t.TempDir()
	itemStore := openItemStore(t, dataDir)
	if _, err := itemStore.AppendMany(context.Background(), []session.Item{
		{SessionID: "s-items", TurnID: "old", Kind: session.ItemMessage, Role: "user", Text: "previous prompt"},
		{SessionID: "s-items", TurnID: "old", Kind: session.ItemMessage, Role: "assistant", Text: "previous answer"},
	}); err != nil {
		t.Fatalf("append prior items: %v", err)
	}
	script := &scriptedProvider{streams: [][]provider.Event{{
		{Kind: provider.EventDelta, Text: "done"},
		{Kind: provider.EventCompleted, Text: "done"},
	}}}
	runner := NewRunner(Config{
		Bus:      eventbus.New(eventbus.WithSubscriberBuffer(16)),
		Items:    itemStore,
		Provider: script,
	})
	defer runner.bus.Close()

	if _, err := runner.Run(context.Background(), RunRequest{SessionID: "s-items", RunID: "new", Prompt: "next prompt", Resume: true}); err != nil {
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

func TestRunnerWithoutResumeIgnoresPriorItems(t *testing.T) {
	dataDir := t.TempDir()
	itemStore := openItemStore(t, dataDir)
	if _, err := itemStore.Append(context.Background(), session.Item{SessionID: "s-items", TurnID: "old", Kind: session.ItemMessage, Role: "user", Text: "previous prompt"}); err != nil {
		t.Fatalf("append prior item: %v", err)
	}
	script := &scriptedProvider{streams: [][]provider.Event{{
		{Kind: provider.EventDelta, Text: "done"},
		{Kind: provider.EventCompleted, Text: "done"},
	}}}
	runner := NewRunner(Config{
		Bus:      eventbus.New(eventbus.WithSubscriberBuffer(16)),
		Items:    itemStore,
		Provider: script,
	})
	defer runner.bus.Close()

	if _, err := runner.Run(context.Background(), RunRequest{SessionID: "s-items", RunID: "new", Prompt: "fresh prompt"}); err != nil {
		t.Fatalf("run: %v", err)
	}
	got := script.requests[0].Messages
	if len(got) != 1 || got[0].Content != "fresh prompt" {
		t.Fatalf("messages = %#v", got)
	}
}

func TestRunnerPersistsTurnResponseIDAndProviderItems(t *testing.T) {
	dataDir := t.TempDir()
	turnStore := openTurnStore(t, dataDir)
	itemStore := openItemStore(t, dataDir)
	runner := NewRunner(Config{
		Bus:   eventbus.New(eventbus.WithSubscriberBuffer(16)),
		Turns: turnStore,
		Items: itemStore,
		Provider: &scriptedProvider{streams: [][]provider.Event{{
			{Kind: provider.EventDelta, Text: "done"},
			{
				Kind:       provider.EventCompleted,
				Text:       "done",
				ResponseID: "resp_123",
				Items: []provider.Item{{
					ID:      "msg_123",
					Kind:    provider.ItemMessage,
					Role:    "assistant",
					Text:    "done",
					RawJSON: []byte(`{"id":"msg_123","type":"message","role":"assistant"}`),
				}},
			},
		}}},
	})
	defer runner.bus.Close()

	if _, err := runner.Run(context.Background(), RunRequest{SessionID: "s-persist", RunID: "r-persist", Prompt: "hello"}); err != nil {
		t.Fatalf("run: %v", err)
	}
	turns, err := turnStore.ListBySession(context.Background(), "s-persist")
	if err != nil {
		t.Fatalf("turns: %v", err)
	}
	if len(turns) != 1 || turns[0].Status != session.TurnStatusCompleted || turns[0].ResponseID != "resp_123" {
		t.Fatalf("turns = %#v", turns)
	}
	items, err := itemStore.ListBySession(context.Background(), "s-persist")
	if err != nil {
		t.Fatalf("items: %v", err)
	}
	if len(items) != 2 {
		t.Fatalf("items = %#v", items)
	}
	if items[0].Role != "user" || items[0].Text != "hello" {
		t.Fatalf("user item = %#v", items[0])
	}
	if items[1].ID != "msg_123" || items[1].Role != "assistant" || !strings.Contains(string(items[1].RawJSON), `"type":"message"`) {
		t.Fatalf("assistant item = %#v", items[1])
	}
}

func TestRunnerCompactionWritesCanonicalItemWindow(t *testing.T) {
	dataDir := t.TempDir()
	itemStore := openItemStore(t, dataDir)
	if _, err := itemStore.Append(context.Background(), session.Item{
		SessionID: "s-compact-items",
		TurnID:    "old",
		Kind:      session.ItemMessage,
		Role:      "user",
		Text:      strings.Repeat("large context ", 80),
	}); err != nil {
		t.Fatalf("append old item: %v", err)
	}
	compactProvider := &compactingCaptureProvider{
		captureProvider: captureProvider{name: "openai", finalText: "done"},
		output:          []json.RawMessage{json.RawMessage(`{"type":"compaction","encrypted_content":"opaque"}`)},
	}
	runner := NewRunner(Config{
		Bus:                   eventbus.New(eventbus.WithSubscriberBuffer(16)),
		Items:                 itemStore,
		Provider:              compactProvider,
		Model:                 "gpt-5.5",
		AutoCompactTokenLimit: 50,
	})
	defer runner.bus.Close()

	if _, err := runner.Run(context.Background(), RunRequest{SessionID: "s-compact-items", RunID: "r-compact", Prompt: "continue", Resume: true}); err != nil {
		t.Fatalf("run: %v", err)
	}
	got := compactProvider.request.Messages
	if len(got) != 2 {
		t.Fatalf("provider messages = %#v", got)
	}
	if len(got[0].RawJSON) == 0 || !strings.Contains(string(got[0].RawJSON), `"encrypted_content":"opaque"`) {
		t.Fatalf("first provider message should be compacted raw item: %#v", got[0])
	}
	if got[1].Role != provider.RoleUser || got[1].Content != "continue" {
		t.Fatalf("suffix prompt = %#v", got[1])
	}

	items, err := itemStore.ListBySession(context.Background(), "s-compact-items")
	if err != nil {
		t.Fatalf("items: %v", err)
	}
	canonical := providerMessagesFromSessionItems(items)
	if len(canonical) < 3 || len(canonical[0].RawJSON) == 0 || canonical[1].Content != "continue" || canonical[2].Content != "done" {
		t.Fatalf("canonical items = %#v", canonical)
	}
}

func openItemStore(t *testing.T, dataDir string) *session.ItemStore {
	t.Helper()
	store, err := session.OpenItemStore(dataDir)
	if err != nil {
		t.Fatalf("item store: %v", err)
	}
	return store
}

func openTurnStore(t *testing.T, dataDir string) *session.TurnStore {
	t.Helper()
	store, err := session.OpenTurnStore(dataDir)
	if err != nil {
		t.Fatalf("turn store: %v", err)
	}
	return store
}
