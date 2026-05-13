package session

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestItemStoreAppendReadKindsAndReopen(t *testing.T) {
	ctx := context.Background()
	dataDir := t.TempDir()
	store := openTestItemStore(t, dataDir, time.Date(2026, 5, 13, 10, 0, 0, 0, time.UTC))

	rawUnknown := json.RawMessage(`{"type":"unknown_shape","opaque":true}`)
	items, err := store.AppendMany(ctx, []Item{
		{SessionID: "s1", TurnID: "t1", Kind: ItemMessage, Role: "assistant", Text: "hello"},
		{SessionID: "s1", TurnID: "t1", Kind: ItemFunctionCall, ToolName: "read_file", ToolCallID: "call_1", RawJSON: json.RawMessage(`{"type":"function_call"}`)},
		{SessionID: "s1", TurnID: "t1", Kind: ItemFunctionOut, ToolCallID: "call_1", Text: "result"},
		{SessionID: "s1", TurnID: "t1", Kind: ItemReasoning, Text: "thinking"},
		{SessionID: "s1", TurnID: "t1", Kind: ItemCompaction, RawJSON: json.RawMessage(`{"type":"compaction"}`)},
		{SessionID: "s1", TurnID: "t2", Kind: ItemRaw, RawJSON: rawUnknown},
	})
	if err != nil {
		t.Fatalf("append many: %v", err)
	}
	if len(items) != 6 || items[0].ID == "" || items[0].CreatedAt.IsZero() {
		t.Fatalf("items = %#v", items)
	}

	reopened, err := OpenItemStore(dataDir)
	if err != nil {
		t.Fatalf("reopen: %v", err)
	}
	all, err := reopened.ListBySession(ctx, "s1")
	if err != nil {
		t.Fatalf("list session: %v", err)
	}
	if len(all) != 6 {
		t.Fatalf("all items = %#v", all)
	}
	if got := []ItemKind{all[0].Kind, all[1].Kind, all[2].Kind, all[3].Kind, all[4].Kind, all[5].Kind}; got[0] != ItemMessage || got[5] != ItemRaw {
		t.Fatalf("item kinds = %#v", got)
	}
	if string(all[5].RawJSON) != string(rawUnknown) {
		t.Fatalf("raw item = %s", all[5].RawJSON)
	}

	turnItems, err := reopened.ListByTurn(ctx, "s1", "t1")
	if err != nil {
		t.Fatalf("list turn: %v", err)
	}
	if len(turnItems) != 5 {
		t.Fatalf("turn items = %#v", turnItems)
	}

	raw, err := os.ReadFile(filepath.Join(dataDir, "sessions", "s1", itemsFileName))
	if err != nil {
		t.Fatalf("read items journal: %v", err)
	}
	if lines := strings.Count(strings.TrimSpace(string(raw)), "\n") + 1; lines != 6 {
		t.Fatalf("append-only item lines = %d\n%s", lines, raw)
	}
}

func openTestItemStore(t *testing.T, dataDir string, now time.Time) *ItemStore {
	t.Helper()
	store, err := OpenItemStore(dataDir)
	if err != nil {
		t.Fatalf("open item store: %v", err)
	}
	store.now = fixedClock(now)
	return store
}
