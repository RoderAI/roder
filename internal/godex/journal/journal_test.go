package journal

import (
	"context"
	"path/filepath"
	"testing"

	"github.com/pandelisz/gode/internal/godex/eventbus"
)

func TestJSONLStoreAppendsAndReplaysSessionEvents(t *testing.T) {
	path := filepath.Join(t.TempDir(), "events.jsonl")
	store, err := Open(path)
	if err != nil {
		t.Fatalf("open: %v", err)
	}

	events := []eventbus.Event{
		{Seq: 1, ID: "e1", SessionID: "s1", Kind: eventbus.KindRunStarted},
		{Seq: 2, ID: "e2", SessionID: "s2", Kind: eventbus.KindRunStarted},
		{Seq: 3, ID: "e3", SessionID: "s1", Kind: eventbus.KindAssistantCompleted},
	}
	for _, ev := range events {
		if err := store.Append(context.Background(), ev); err != nil {
			t.Fatalf("append: %v", err)
		}
	}
	if err := store.Close(); err != nil {
		t.Fatalf("close: %v", err)
	}

	reopened, err := Open(path)
	if err != nil {
		t.Fatalf("reopen: %v", err)
	}
	defer reopened.Close()

	got, err := reopened.Replay(context.Background(), ReplayFilter{SessionID: "s1"})
	if err != nil {
		t.Fatalf("replay: %v", err)
	}
	if len(got) != 2 {
		t.Fatalf("len = %d, want 2", len(got))
	}
	if got[0].ID != "e1" || got[1].ID != "e3" {
		t.Fatalf("ids = %q, %q", got[0].ID, got[1].ID)
	}

	sessions, err := reopened.ListSessions(context.Background())
	if err != nil {
		t.Fatalf("sessions: %v", err)
	}
	if len(sessions) != 2 || sessions[0] != "s1" || sessions[1] != "s2" {
		t.Fatalf("sessions = %#v", sessions)
	}
}
