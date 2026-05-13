package main

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/godex/journal"
)

func TestSessionDoctorAndRepairCommands(t *testing.T) {
	dataDir := t.TempDir()
	workspace := t.TempDir()
	journalPath := filepath.Join(dataDir, "events.jsonl")
	store, err := journal.Open(journalPath)
	if err != nil {
		t.Fatalf("open journal: %v", err)
	}
	events := []eventbus.Event{
		{ID: "run", SessionID: "s1", RunID: "r1", Source: eventbus.SourceAgent, Kind: eventbus.KindRunStarted},
		{ID: "user", SessionID: "s1", RunID: "r1", Source: eventbus.SourceTUI, Kind: eventbus.KindUserPromptSubmitted, Payload: map[string]any{"prompt": "repair"}},
		{ID: "done", SessionID: "s1", RunID: "r1", Source: eventbus.SourceAgent, Kind: eventbus.KindRunCompleted},
	}
	for _, ev := range events {
		if err := store.Append(context.Background(), ev); err != nil {
			t.Fatalf("append: %v", err)
		}
	}
	if err := store.Close(); err != nil {
		t.Fatalf("close: %v", err)
	}
	if err := os.MkdirAll(filepath.Join(dataDir, "sessions", "s1"), 0o700); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if err := os.WriteFile(filepath.Join(dataDir, "sessions", "s1", "items.jsonl"), []byte("{bad json}\n"), 0o600); err != nil {
		t.Fatalf("write bad items: %v", err)
	}

	doctorOut := captureStdout(t, func() error {
		return runSession([]string{"doctor", "--workspace", workspace, "--data-dir", dataDir, "--provider", "mock"})
	})
	if !strings.Contains(doctorOut, "invalid_items\t1") || !strings.Contains(doctorOut, "items.jsonl:1") {
		t.Fatalf("doctor output:\n%s", doctorOut)
	}

	repairOut := captureStdout(t, func() error {
		return runSession([]string{"repair", "--from-journal", "--workspace", workspace, "--data-dir", dataDir, "--provider", "mock"})
	})
	if !strings.Contains(repairOut, "repair_actions") || !strings.Contains(repairOut, "index.json.repaired") {
		t.Fatalf("repair output:\n%s", repairOut)
	}
	if _, err := os.Stat(filepath.Join(dataDir, "sessions", "index.json.repaired")); err != nil {
		t.Fatalf("missing repaired index: %v", err)
	}
}
