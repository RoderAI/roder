package session

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"testing"
	"time"
)

func TestStoreCreateGetListReopenAndLast(t *testing.T) {
	ctx := context.Background()
	dataDir := t.TempDir()
	store := openTestStore(t, dataDir, time.Date(2026, 5, 13, 10, 0, 0, 0, time.UTC))

	first, err := store.Create(ctx, "First session", "")
	if err != nil {
		t.Fatalf("create first: %v", err)
	}
	store.now = fixedClock(time.Date(2026, 5, 13, 11, 0, 0, 0, time.UTC))
	second, err := store.Create(ctx, "Second session", first.ID)
	if err != nil {
		t.Fatalf("create second: %v", err)
	}
	if first.ID == "" || second.ID == "" || first.ID == second.ID {
		t.Fatalf("ids = %q %q", first.ID, second.ID)
	}

	got, ok, err := store.Get(ctx, first.ID)
	if err != nil {
		t.Fatalf("get: %v", err)
	}
	if !ok || got.Title != "First session" || got.ParentSessionID != "" {
		t.Fatalf("first session = %#v ok=%v", got, ok)
	}

	list, err := store.List(ctx)
	if err != nil {
		t.Fatalf("list: %v", err)
	}
	if len(list) != 2 || list[0].ID != second.ID || list[1].ID != first.ID {
		t.Fatalf("list order = %#v", list)
	}
	last, ok, err := store.Last(ctx)
	if err != nil {
		t.Fatalf("last: %v", err)
	}
	if !ok || last.ID != second.ID {
		t.Fatalf("last = %#v ok=%v", last, ok)
	}

	reopened, err := Open(dataDir)
	if err != nil {
		t.Fatalf("reopen: %v", err)
	}
	reopenedList, err := reopened.List(ctx)
	if err != nil {
		t.Fatalf("reopened list: %v", err)
	}
	if len(reopenedList) != 2 || reopenedList[0].ID != second.ID || reopenedList[1].ID != first.ID {
		t.Fatalf("reopened list = %#v", reopenedList)
	}
}

func TestStoreRenameDeleteAndJournalIndependence(t *testing.T) {
	ctx := context.Background()
	dataDir := t.TempDir()
	store := openTestStore(t, dataDir, time.Date(2026, 5, 13, 10, 0, 0, 0, time.UTC))
	session, err := store.Create(ctx, "Original", "")
	if err != nil {
		t.Fatalf("create: %v", err)
	}
	journalPath := filepath.Join(dataDir, "events.jsonl")
	if err := os.WriteFile(journalPath, []byte("journal\n"), 0o600); err != nil {
		t.Fatalf("write journal: %v", err)
	}

	store.now = fixedClock(time.Date(2026, 5, 13, 12, 0, 0, 0, time.UTC))
	renamed, err := store.Rename(ctx, session.ID, "Renamed")
	if err != nil {
		t.Fatalf("rename: %v", err)
	}
	if renamed.Title != "Renamed" || !renamed.UpdatedAt.After(session.UpdatedAt) {
		t.Fatalf("renamed = %#v", renamed)
	}

	if err := store.Delete(ctx, session.ID); err != nil {
		t.Fatalf("delete: %v", err)
	}
	if _, ok, err := store.Get(ctx, session.ID); err != nil || ok {
		t.Fatalf("deleted get ok=%v err=%v", ok, err)
	}
	if _, err := os.Stat(journalPath); err != nil {
		t.Fatalf("journal should remain after session delete: %v", err)
	}
	if err := store.Delete(ctx, session.ID); !errors.Is(err, ErrNotFound) {
		t.Fatalf("delete missing err = %v", err)
	}
}

func TestStoreLastEmpty(t *testing.T) {
	store := openTestStore(t, t.TempDir(), time.Now())
	last, ok, err := store.Last(context.Background())
	if err != nil {
		t.Fatalf("last: %v", err)
	}
	if ok || last.ID != "" {
		t.Fatalf("last = %#v ok=%v", last, ok)
	}
}

func openTestStore(t *testing.T, dataDir string, now time.Time) *Store {
	t.Helper()
	store, err := Open(dataDir)
	if err != nil {
		t.Fatalf("open: %v", err)
	}
	store.now = fixedClock(now)
	return store
}

func fixedClock(now time.Time) func() time.Time {
	return func() time.Time { return now }
}
