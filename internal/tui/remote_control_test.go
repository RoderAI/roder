package tui

import (
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"
)

func TestRemoteCommandOpensPanel(t *testing.T) {
	model := New(nil)
	model.input.SetValue("/remote")
	updated, _ := model.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	got := updated.(Model)
	if !got.remoteOpen {
		t.Fatal("remote panel should open")
	}
	vm := got.viewModel()
	if vm.Remote == nil || vm.Remote.Title != "Remote Control" {
		t.Fatalf("remote view model = %#v", vm.Remote)
	}
}

func TestSettingsOpensRemotePanel(t *testing.T) {
	model := New(nil)
	model.openSettings()
	model.settings.MenuIndex = settingsMenuIndex(t, model.settings, "remote-control")

	updated, _ := model.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	got := updated.(Model)
	if got.settings.Open {
		t.Fatal("settings should close when remote panel opens")
	}
	if !got.remoteOpen {
		t.Fatal("remote panel should open from settings")
	}
}

func TestRemoteDialogRendersStoppedState(t *testing.T) {
	model := New(nil)
	model.width = 100
	model.height = 30
	model.openRemotePanel()
	view := model.View().Content
	for _, want := range []string{"Remote Control", "stopped", "enter start/stop"} {
		if !strings.Contains(view, want) {
			t.Fatalf("remote view missing %q:\n%s", want, view)
		}
	}
}
