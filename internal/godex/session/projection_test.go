package session

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/godex/journal"
)

func TestProjectionFromEventProjectsCoreItems(t *testing.T) {
	events := []eventbus.Event{
		projectionEvent("e1", eventbus.KindUserPromptSubmitted, map[string]any{"prompt": "hi"}),
		projectionEvent("e2", eventbus.KindAssistantCompleted, map[string]any{"text": "hello"}),
		projectionEvent("e3", eventbus.KindReasoningSummaryCompleted, map[string]any{"text": "reason"}),
		projectionEvent("e4", eventbus.KindToolRequested, map[string]any{"tool": "read_file", "tool_call_id": "call_1", "input": map[string]any{"path": "README.md"}}),
		projectionEvent("e5", eventbus.KindToolCompleted, map[string]any{"tool": "read_file", "tool_call_id": "call_1", "text": "contents"}),
		projectionEvent("e6", eventbus.KindContextCompactionCompleted, map[string]any{"output_items": 2}),
	}
	var kinds []ItemKind
	for _, ev := range events {
		items := ProjectionFromEvent(ev)
		if len(items) != 1 {
			t.Fatalf("%s projected %d items", ev.Kind, len(items))
		}
		kinds = append(kinds, items[0].Kind)
	}
	want := []ItemKind{ItemMessage, ItemMessage, ItemReasoning, ItemFunctionCall, ItemFunctionOut, ItemCompaction}
	for i := range want {
		if kinds[i] != want[i] {
			t.Fatalf("kinds = %#v", kinds)
		}
	}
}

func TestBackfillProjectsJournalAndCoalescesAssistantDeltas(t *testing.T) {
	ctx := context.Background()
	dataDir := t.TempDir()
	journalStore := openProjectionJournal(t, filepath.Join(dataDir, "events.jsonl"))
	events := []eventbus.Event{
		projectionEvent("run", eventbus.KindRunStarted, nil),
		projectionEvent("user", eventbus.KindUserPromptSubmitted, map[string]any{"prompt": "hi"}),
		projectionEvent("d1", eventbus.KindAssistantDelta, map[string]any{"text": "hel"}),
		projectionEvent("d2", eventbus.KindAssistantDelta, map[string]any{"text": "lo"}),
		projectionEvent("done", eventbus.KindRunCompleted, nil),
	}
	for _, ev := range events {
		if err := journalStore.Append(ctx, ev); err != nil {
			t.Fatalf("append journal: %v", err)
		}
	}
	if err := journalStore.Flush(); err != nil {
		t.Fatalf("flush journal: %v", err)
	}

	sessionStore, err := Open(dataDir)
	if err != nil {
		t.Fatalf("open sessions: %v", err)
	}
	turnStore, err := OpenTurnStore(dataDir)
	if err != nil {
		t.Fatalf("open turns: %v", err)
	}
	itemStore, err := OpenItemStore(dataDir)
	if err != nil {
		t.Fatalf("open items: %v", err)
	}
	bus := eventbus.New()
	ch := bus.Subscribe(ctx, eventbus.Filter{Kinds: []eventbus.Kind{eventbus.KindSessionProjected}})

	result, err := Backfill(ctx, journalStore, BackfillStores{Sessions: sessionStore, Turns: turnStore, Items: itemStore, Bus: bus})
	if err != nil {
		t.Fatalf("backfill: %v", err)
	}
	if result.Sessions != 1 || result.Turns != 1 || result.Items != 2 {
		t.Fatalf("result = %#v", result)
	}
	projected := <-ch
	if projected.Kind != eventbus.KindSessionProjected {
		t.Fatalf("projected event = %#v", projected)
	}

	sessions, err := sessionStore.List(ctx)
	if err != nil {
		t.Fatalf("list sessions: %v", err)
	}
	if len(sessions) != 1 || sessions[0].ID != "s1" {
		t.Fatalf("sessions = %#v", sessions)
	}
	turns, err := turnStore.ListBySession(ctx, "s1")
	if err != nil {
		t.Fatalf("list turns: %v", err)
	}
	if len(turns) != 1 || turns[0].Status != TurnStatusCompleted {
		t.Fatalf("turns = %#v", turns)
	}
	items, err := itemStore.ListBySession(ctx, "s1")
	if err != nil {
		t.Fatalf("list items: %v", err)
	}
	if len(items) != 2 || items[0].Text != "hi" || items[1].Text != "hello" {
		t.Fatalf("items = %#v", items)
	}

	again, err := Backfill(ctx, journalStore, BackfillStores{Sessions: sessionStore, Turns: turnStore, Items: itemStore})
	if err != nil {
		t.Fatalf("second backfill: %v", err)
	}
	if again.Items != 0 {
		t.Fatalf("second backfill should be idempotent, got %#v", again)
	}
}

func TestBackfillReportsCorruptJournalPathAndLine(t *testing.T) {
	ctx := context.Background()
	dataDir := t.TempDir()
	path := filepath.Join(dataDir, "events.jsonl")
	if err := writeCorruptJournal(path); err != nil {
		t.Fatalf("write corrupt journal: %v", err)
	}
	journalStore := openProjectionJournal(t, path)
	sessionStore, _ := Open(dataDir)
	turnStore, _ := OpenTurnStore(dataDir)
	itemStore, _ := OpenItemStore(dataDir)
	_, err := Backfill(ctx, journalStore, BackfillStores{Sessions: sessionStore, Turns: turnStore, Items: itemStore})
	if err == nil || !strings.Contains(err.Error(), path+":2") {
		t.Fatalf("corrupt journal err = %v", err)
	}
}

func projectionEvent(id string, kind eventbus.Kind, payload any) eventbus.Event {
	return eventbus.Event{
		ID:        id,
		Seq:       1,
		Time:      time.Date(2026, 5, 13, 10, 0, 0, 0, time.UTC),
		SessionID: "s1",
		RunID:     "r1",
		Source:    eventbus.SourceAgent,
		Kind:      kind,
		Payload:   payload,
	}
}

func openProjectionJournal(t *testing.T, path string) *journal.Store {
	t.Helper()
	store, err := journal.Open(path)
	if err != nil {
		t.Fatalf("open journal: %v", err)
	}
	t.Cleanup(func() { _ = store.Close() })
	return store
}

func writeCorruptJournal(path string) error {
	store, err := journal.Open(path)
	if err != nil {
		return err
	}
	if err := store.Append(context.Background(), projectionEvent("ok", eventbus.KindRunStarted, nil)); err != nil {
		_ = store.Close()
		return err
	}
	if err := store.Close(); err != nil {
		return err
	}
	file, err := os.OpenFile(path, os.O_WRONLY|os.O_APPEND, 0o600)
	if err != nil {
		return err
	}
	defer file.Close()
	_, err = file.WriteString("{bad json}\n")
	return err
}
