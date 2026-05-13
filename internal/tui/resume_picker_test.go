package tui

import (
	"context"
	"path/filepath"
	"strings"
	"testing"
	"time"

	tea "charm.land/bubbletea/v2"
	"github.com/pandelisz/gode/internal/godex"
	"github.com/pandelisz/gode/internal/godex/session"
)

func TestResumePickerFiltersCurrentDirectoryAndSearches(t *testing.T) {
	current := filepath.Join(t.TempDir(), "current")
	other := filepath.Join(t.TempDir(), "other")
	model := resumePickerModel{
		workspace:    normalizeWorkspace(current),
		scopeCurrent: true,
		sessions: []session.Session{
			{ID: "s-current", Title: "Current feature", Workspace: current, MessageCount: 4, UpdatedAt: time.Now()},
			{ID: "s-other", Title: "Other feature", Workspace: other, MessageCount: 2, UpdatedAt: time.Now()},
		},
	}

	items := model.filtered()
	if len(items) != 1 || items[0].ID != "s-current" {
		t.Fatalf("current filtered items = %#v", items)
	}

	model.scopeCurrent = false
	model.query = "other"
	items = model.filtered()
	if len(items) != 1 || items[0].ID != "s-other" {
		t.Fatalf("searched items = %#v", items)
	}
}

func TestResumePickerKeyboardTogglesScopeAndChoosesSession(t *testing.T) {
	current := filepath.Join(t.TempDir(), "current")
	model := resumePickerModel{
		workspace:    normalizeWorkspace(current),
		scopeCurrent: true,
		sessions: []session.Session{
			{ID: "s-current", Title: "Current feature", Workspace: current, UpdatedAt: time.Now()},
			{ID: "s-all", Title: "All feature", Workspace: filepath.Join(t.TempDir(), "other"), UpdatedAt: time.Now()},
		},
	}

	updated, _ := model.Update(tea.KeyPressMsg{Code: tea.KeyTab})
	got := updated.(resumePickerModel)
	if got.scopeCurrent {
		t.Fatal("tab should switch to all sessions")
	}
	for _, r := range []rune("all") {
		updated, _ = got.Update(tea.KeyPressMsg{Code: r})
		got = updated.(resumePickerModel)
	}
	if got.query != "all" {
		t.Fatalf("query = %q", got.query)
	}
	updated, _ = got.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	got = updated.(resumePickerModel)
	if got.chosenID != "s-all" {
		t.Fatalf("chosen id = %q", got.chosenID)
	}
}

func TestResumePickerArrowKeysMoveSelection(t *testing.T) {
	current := filepath.Join(t.TempDir(), "current")
	model := resumePickerModel{
		workspace:    normalizeWorkspace(current),
		scopeCurrent: true,
		sessions: []session.Session{
			{ID: "s-first", Title: "First", Workspace: current, UpdatedAt: time.Now()},
			{ID: "s-second", Title: "Second", Workspace: current, UpdatedAt: time.Now()},
		},
	}

	updated, _ := model.Update(tea.KeyPressMsg{Code: tea.KeyDown})
	got := updated.(resumePickerModel)
	if got.selected != 1 {
		t.Fatalf("selected after down = %d, want 1", got.selected)
	}

	updated, _ = got.Update(tea.KeyPressMsg{Code: tea.KeyUp})
	got = updated.(resumePickerModel)
	if got.selected != 0 {
		t.Fatalf("selected after up = %d, want 0", got.selected)
	}
}

func TestResumePickerShowsCurrentAllControls(t *testing.T) {
	workspace := t.TempDir()
	model := resumePickerModel{
		scopeCurrent: true,
		workspace:    normalizeWorkspace(workspace),
		sessions: []session.Session{{
			ID:           "s1",
			Title:        "Feature work",
			Workspace:    workspace,
			MessageCount: 3,
			UpdatedAt:    time.Date(2026, 5, 13, 20, 0, 0, 0, time.UTC),
		}},
	}

	view := model.viewString()
	for _, want := range []string{"gode resume", "search:", "current dir", "all", "Feature work", "enter resume"} {
		if !strings.Contains(view, want) {
			t.Fatalf("view missing %q:\n%s", want, view)
		}
	}
	if strings.Count(view, "\n") > 12 {
		t.Fatalf("picker should stay compact, got %d lines:\n%s", strings.Count(view, "\n")+1, view)
	}
}

func TestRunnerPersistsSessionWorkspaceForResumeFilter(t *testing.T) {
	workspace := t.TempDir()
	app, err := godex.New(context.Background(), godex.Config{
		DataDir:     t.TempDir(),
		Workspace:   workspace,
		Provider:    "mock",
		Model:       "mock",
		Reasoning:   "none",
		AutoApprove: true,
	})
	if err != nil {
		t.Fatalf("app: %v", err)
	}
	defer app.Close(context.Background())

	result, err := app.RunPrompt(context.Background(), "hello")
	if err != nil {
		t.Fatalf("run prompt: %v", err)
	}
	item, ok, err := app.Sessions.Get(context.Background(), result.SessionID)
	if err != nil {
		t.Fatalf("get session: %v", err)
	}
	if !ok {
		t.Fatal("session not found")
	}
	if normalizeWorkspace(item.Workspace) != normalizeWorkspace(workspace) || item.Model != "mock" || item.Provider != "mock" {
		t.Fatalf("session metadata = %#v", item)
	}
}
