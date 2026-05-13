package tui

import (
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

func assertNotQuitCmd(t *testing.T, cmd tea.Cmd) {
	t.Helper()
	if cmd == nil {
		return
	}
	if _, ok := cmd().(tea.QuitMsg); ok {
		t.Fatal("command should not quit")
	}
}
