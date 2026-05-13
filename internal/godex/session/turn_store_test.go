package session

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestTurnStoreAppendCompleteFailListAndReopen(t *testing.T) {
	ctx := context.Background()
	dataDir := t.TempDir()
	store := openTestTurnStore(t, dataDir, time.Date(2026, 5, 13, 10, 0, 0, 0, time.UTC))

	first, err := store.Append(ctx, Turn{
		SessionID: "session-a",
		Prompt:    "first prompt",
		Model:     "gpt-5.5",
		Provider:  "codex",
	})
	if err != nil {
		t.Fatalf("append first: %v", err)
	}
	if first.ID == "" || first.Status != TurnStatusRunning || first.StartedAt.IsZero() {
		t.Fatalf("first turn = %#v", first)
	}

	store.now = fixedClock(time.Date(2026, 5, 13, 10, 1, 0, 0, time.UTC))
	completed, err := store.Complete(ctx, "session-a", first.ID, "resp_123")
	if err != nil {
		t.Fatalf("complete first: %v", err)
	}
	if completed.Status != TurnStatusCompleted || completed.ResponseID != "resp_123" || completed.CompletedAt.IsZero() {
		t.Fatalf("completed turn = %#v", completed)
	}

	store.now = fixedClock(time.Date(2026, 5, 13, 10, 2, 0, 0, time.UTC))
	second, err := store.Append(ctx, Turn{
		ID:        "turn-2",
		SessionID: "session-a",
		Prompt:    "second prompt",
		Status:    TurnStatusRunning,
	})
	if err != nil {
		t.Fatalf("append second: %v", err)
	}
	store.now = fixedClock(time.Date(2026, 5, 13, 10, 3, 0, 0, time.UTC))
	failed, err := store.Fail(ctx, "session-a", second.ID, "model stopped")
	if err != nil {
		t.Fatalf("fail second: %v", err)
	}
	if failed.Status != TurnStatusFailed || failed.Error != "model stopped" {
		t.Fatalf("failed turn = %#v", failed)
	}

	list, err := store.ListBySession(ctx, "session-a")
	if err != nil {
		t.Fatalf("list: %v", err)
	}
	if len(list) != 2 {
		t.Fatalf("turn count = %d %#v", len(list), list)
	}
	if list[0].ID != second.ID || list[1].ID != first.ID {
		t.Fatalf("turn order = %#v", list)
	}

	reopened, err := OpenTurnStore(dataDir)
	if err != nil {
		t.Fatalf("reopen: %v", err)
	}
	reopenedList, err := reopened.ListBySession(ctx, "session-a")
	if err != nil {
		t.Fatalf("reopened list: %v", err)
	}
	if len(reopenedList) != 2 || reopenedList[0].Status != TurnStatusFailed || reopenedList[1].ResponseID != "resp_123" {
		t.Fatalf("reopened list = %#v", reopenedList)
	}

	raw, err := os.ReadFile(filepath.Join(dataDir, "sessions", "session-a", turnsFileName))
	if err != nil {
		t.Fatalf("read turns journal: %v", err)
	}
	if lines := strings.Count(strings.TrimSpace(string(raw)), "\n") + 1; lines != 4 {
		t.Fatalf("append-only journal lines = %d\n%s", lines, raw)
	}
}

func TestTurnStoreListMissingSessionAndNotFound(t *testing.T) {
	ctx := context.Background()
	store := openTestTurnStore(t, t.TempDir(), time.Date(2026, 5, 13, 10, 0, 0, 0, time.UTC))

	list, err := store.ListBySession(ctx, "missing")
	if err != nil {
		t.Fatalf("list missing: %v", err)
	}
	if len(list) != 0 {
		t.Fatalf("missing list = %#v", list)
	}
	if _, err := store.Complete(ctx, "missing", "turn", "resp"); err != ErrNotFound {
		t.Fatalf("complete missing err = %v", err)
	}
}

func openTestTurnStore(t *testing.T, dataDir string, now time.Time) *TurnStore {
	t.Helper()
	store, err := OpenTurnStore(dataDir)
	if err != nil {
		t.Fatalf("open turn store: %v", err)
	}
	store.now = fixedClock(now)
	return store
}
