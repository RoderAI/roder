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

func TestRepairDoctorReportsMissingIndexMissingTurnAndInvalidItemJSON(t *testing.T) {
	ctx := context.Background()
	dataDir := t.TempDir()
	itemStore, err := OpenItemStore(dataDir)
	if err != nil {
		t.Fatalf("open items: %v", err)
	}
	if _, err := itemStore.Append(ctx, Item{SessionID: "s-missing", TurnID: "turn-missing", Kind: ItemMessage, Role: "user", Text: "hello"}); err != nil {
		t.Fatalf("append item: %v", err)
	}
	badDir := filepath.Join(dataDir, "sessions", "s-bad")
	if err := os.MkdirAll(badDir, 0o700); err != nil {
		t.Fatalf("mkdir bad: %v", err)
	}
	if err := os.WriteFile(filepath.Join(badDir, itemsFileName), []byte("{\"id\":\"ok\",\"session_id\":\"s-bad\",\"kind\":\"message\"}\n{bad json}\n"), 0o600); err != nil {
		t.Fatalf("write bad items: %v", err)
	}

	report, err := Doctor(ctx, dataDir)
	if err != nil {
		t.Fatalf("doctor: %v", err)
	}
	if report.MissingIndex != 2 {
		t.Fatalf("missing index = %d report=%#v", report.MissingIndex, report)
	}
	if report.MissingTurns != 1 {
		t.Fatalf("missing turns = %d report=%#v", report.MissingTurns, report)
	}
	if report.InvalidItems != 1 {
		t.Fatalf("invalid items = %d report=%#v", report.InvalidItems, report)
	}
	if !containsDiagnostic(report.Diagnostics, filepath.Join(dataDir, "sessions", "s-bad", itemsFileName)+":2") {
		t.Fatalf("diagnostics missing item line: %#v", report.Diagnostics)
	}
}

func TestRepairFromJournalWritesRepairedFilesBesideOriginal(t *testing.T) {
	ctx := context.Background()
	dataDir := t.TempDir()
	journalPath := filepath.Join(dataDir, "events.jsonl")
	store, err := journal.Open(journalPath)
	if err != nil {
		t.Fatalf("open journal: %v", err)
	}
	events := []eventbus.Event{
		repairEvent("run", eventbus.KindRunStarted, nil),
		repairEvent("user", eventbus.KindUserPromptSubmitted, map[string]any{"prompt": "repair me"}),
		repairEvent("answer", eventbus.KindAssistantCompleted, map[string]any{"text": "done"}),
		repairEvent("complete", eventbus.KindRunCompleted, nil),
	}
	for _, ev := range events {
		if err := store.Append(ctx, ev); err != nil {
			t.Fatalf("append journal: %v", err)
		}
	}
	if err := store.Close(); err != nil {
		t.Fatalf("close journal: %v", err)
	}
	if err := os.MkdirAll(filepath.Join(dataDir, "sessions"), 0o700); err != nil {
		t.Fatalf("mkdir sessions: %v", err)
	}
	if err := os.WriteFile(filepath.Join(dataDir, "sessions", indexFileName), []byte("{damaged}\n"), 0o600); err != nil {
		t.Fatalf("write damaged index: %v", err)
	}

	report, err := RepairFromJournal(ctx, dataDir, journalPath)
	if err != nil {
		t.Fatalf("repair: %v", err)
	}
	if len(report.RepairActions) == 0 {
		t.Fatalf("repair actions empty: %#v", report)
	}
	if _, err := os.Stat(filepath.Join(dataDir, "sessions", indexFileName+".repaired")); err != nil {
		t.Fatalf("missing repaired index: %v", err)
	}
	if _, err := os.Stat(filepath.Join(dataDir, "sessions", "s1", itemsFileName+".repaired")); err != nil {
		t.Fatalf("missing repaired items: %v", err)
	}
	original, err := os.ReadFile(filepath.Join(dataDir, "sessions", indexFileName))
	if err != nil {
		t.Fatalf("read original index: %v", err)
	}
	if string(original) != "{damaged}\n" {
		t.Fatalf("original index was modified: %q", original)
	}
}

func repairEvent(id string, kind eventbus.Kind, payload any) eventbus.Event {
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

func containsDiagnostic(diagnostics []string, needle string) bool {
	for _, diagnostic := range diagnostics {
		if strings.Contains(diagnostic, needle) {
			return true
		}
	}
	return false
}
