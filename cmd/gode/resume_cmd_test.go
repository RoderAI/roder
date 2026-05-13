package main

import (
	"context"
	"path/filepath"
	"testing"

	"github.com/pandelisz/gode/internal/godex"
)

func TestRunResumeStartsResumeTUIWithConfig(t *testing.T) {
	workspace := filepath.Join(t.TempDir(), "workspace")
	dataDir := filepath.Join(t.TempDir(), "data")
	var got godex.Config
	called := false
	previous := runResumeTUI
	runResumeTUI = func(_ context.Context, app *godex.App) error {
		called = true
		got = app.Config
		if app.Sessions == nil {
			t.Fatal("sessions store not wired")
		}
		return nil
	}
	t.Cleanup(func() {
		runResumeTUI = previous
	})

	err := runResume(context.Background(), []string{
		"--workspace", workspace,
		"--data-dir", dataDir,
		"--provider", "mock",
		"--model", "mock",
		"--reasoning", "none",
	})
	if err != nil {
		t.Fatalf("resume: %v", err)
	}
	if !called {
		t.Fatal("resume TUI was not started")
	}
	if got.Workspace != workspace || got.DataDir != dataDir || got.Provider != "mock" || got.Model != "mock" || got.Reasoning != "none" {
		t.Fatalf("config = %#v", got)
	}
}

func TestRunDispatchesResumeCommand(t *testing.T) {
	previous := runResumeTUI
	runResumeTUI = func(context.Context, *godex.App) error { return nil }
	t.Cleanup(func() {
		runResumeTUI = previous
	})

	err := run(context.Background(), []string{"resume", "--data-dir", t.TempDir(), "--provider", "mock", "--model", "mock", "--reasoning", "none"})
	if err != nil {
		t.Fatalf("run resume: %v", err)
	}
}
