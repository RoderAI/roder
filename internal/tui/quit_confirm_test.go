package tui

import (
	"context"
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"
)

func TestEscapeOpensQuitConfirmationInsteadOfQuitting(t *testing.T) {
	model := New(nil)
	updated, cmd := model.Update(tea.KeyPressMsg{Code: tea.KeyEsc})
	got := updated.(Model)

	if cmd != nil {
		t.Fatalf("first escape should not quit, got cmd %#v", cmd())
	}
	if got.viewModel().QuitDialog == nil {
		t.Fatal("expected quit confirmation dialog to open")
	}
	if view := got.View().Content; !strings.Contains(view, "Quit gode?") || !strings.Contains(view, "Enter quit") {
		t.Fatalf("view missing quit confirmation:\n%s", view)
	}
}

func TestQuitConfirmationEnterQuits(t *testing.T) {
	model := New(nil)
	updated, _ := model.Update(tea.KeyPressMsg{Code: tea.KeyEsc})
	got := updated.(Model)

	_, cmd := got.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	if cmd == nil {
		t.Fatal("enter on quit confirmation should return quit command")
	}
	if _, ok := cmd().(tea.QuitMsg); !ok {
		t.Fatalf("enter command = %#v, want tea.QuitMsg", cmd())
	}
}

func TestQuitConfirmationRightCancels(t *testing.T) {
	model := New(nil)
	updated, _ := model.Update(tea.KeyPressMsg{Code: 'c', Mod: tea.ModCtrl})
	got := updated.(Model)

	updated, cmd := got.Update(tea.KeyPressMsg{Code: tea.KeyRight})
	got = updated.(Model)
	assertNotQuitCmd(t, cmd)
	if got.viewModel().QuitDialog != nil {
		t.Fatal("right should close quit confirmation")
	}
}

func TestQuitConfirmationEscapeCancels(t *testing.T) {
	model := New(nil)
	updated, _ := model.Update(tea.KeyPressMsg{Code: 'c', Mod: tea.ModCtrl})
	got := updated.(Model)

	updated, cmd := got.Update(tea.KeyPressMsg{Code: tea.KeyEsc})
	got = updated.(Model)
	assertNotQuitCmd(t, cmd)
	if got.viewModel().QuitDialog != nil {
		t.Fatal("second escape should close quit confirmation")
	}
}

func TestEscapeWhileRunningOpensStopConfirmation(t *testing.T) {
	model := New(nil)
	model.running = true
	updated, cmd := model.Update(tea.KeyPressMsg{Code: tea.KeyEsc})
	got := updated.(Model)

	if cmd != nil {
		t.Fatalf("escape while running should not run a command, got %#v", cmd())
	}
	if got.viewModel().StopDialog == nil {
		t.Fatal("expected stop confirmation dialog to open")
	}
	if got.viewModel().QuitDialog != nil {
		t.Fatal("running escape should not open quit confirmation")
	}
	if view := got.View().Content; !strings.Contains(view, "Stop current turn?") || !strings.Contains(view, "Enter stop") {
		t.Fatalf("view missing stop confirmation:\n%s", view)
	}
}

func TestStopConfirmationEnterCancelsRun(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	model := New(nil)
	model.running = true
	model.runCancel = cancel

	updated, _ := model.Update(tea.KeyPressMsg{Code: tea.KeyEsc})
	got := updated.(Model)
	updated, cmd := got.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	got = updated.(Model)

	assertNotQuitCmd(t, cmd)
	select {
	case <-ctx.Done():
	default:
		t.Fatal("enter on stop confirmation should cancel the active run context")
	}
	if got.viewModel().StopDialog != nil {
		t.Fatal("stop confirmation should close after enter")
	}
}

func TestStopConfirmationEscapeCancelsDialogOnly(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	model := New(nil)
	model.running = true
	model.runCancel = cancel

	updated, _ := model.Update(tea.KeyPressMsg{Code: tea.KeyEsc})
	got := updated.(Model)
	updated, cmd := got.Update(tea.KeyPressMsg{Code: tea.KeyEsc})
	got = updated.(Model)

	assertNotQuitCmd(t, cmd)
	select {
	case <-ctx.Done():
		t.Fatal("second escape should keep the active run alive")
	default:
	}
	if got.viewModel().StopDialog != nil {
		t.Fatal("second escape should close stop confirmation")
	}
}

func assertNotQuitCmd(t *testing.T, cmd tea.Cmd) {
	t.Helper()
	if cmd == nil {
		return
	}
	if _, ok := cmd().(tea.QuitMsg); ok {
		t.Fatal("command should not quit")
	}
}
