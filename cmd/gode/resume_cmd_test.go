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
	previousPicker := pickResumeSession
	previousRunner := runResumeTUI
	pickResumeSession = func(_ context.Context, app *godex.App) (string, error) {
		got = app.Config
		if app.Sessions == nil {
			t.Fatal("sessions store not wired")
		}
		return "session-id", nil
	}
	runResumeTUI = func(_ context.Context, app *godex.App, sessionID string) error {
		called = true
		if sessionID != "session-id" {
			t.Fatalf("session id = %q", sessionID)
		}
		return nil
	}
	t.Cleanup(func() {
		pickResumeSession = previousPicker
		runResumeTUI = previousRunner
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
	previousPicker := pickResumeSession
	previousRunner := runResumeTUI
	pickResumeSession = func(context.Context, *godex.App) (string, error) { return "", nil }
	runResumeTUI = func(context.Context, *godex.App, string) error { return nil }
	t.Cleanup(func() {
		pickResumeSession = previousPicker
		runResumeTUI = previousRunner
	})

	err := run(context.Background(), []string{"resume", "--data-dir", t.TempDir(), "--provider", "mock", "--model", "mock", "--reasoning", "none"})
	if err != nil {
		t.Fatalf("run resume: %v", err)
	}
}
