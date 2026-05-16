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

func TestRemotePanelCopiesURLAndAuthHeader(t *testing.T) {
	model := New(nil)
	model.remoteOpen = true
	model.remoteState.URLs = []string{"ws://100.99.1.2:1234"}
	model.remoteState.AuthHeader = "Authorization: Bearer secret"
	model.remoteState.TokenPreview = "secret-preview"
	model.width = 100
	model.height = 30
	var copied []string
	model.clipboardWrite = func(text string) error {
		copied = append(copied, text)
		return nil
	}

	updated, cmd := model.Update(tea.KeyPressMsg{Code: 'u', Text: "u"})
	got := updated.(Model)
	if cmd == nil {
		t.Fatal("copy url command missing")
	}
	updated, _ = got.Update(cmd())
	got = updated.(Model)
	if len(copied) != 1 || copied[0] != "ws://100.99.1.2:1234" || got.status != "Copied remote URL" {
		t.Fatalf("url copy copied=%#v status=%q", copied, got.status)
	}

	updated, cmd = got.Update(tea.KeyPressMsg{Code: 'h', Text: "h"})
	got = updated.(Model)
	if cmd == nil {
		t.Fatal("copy auth command missing")
	}
	updated, _ = got.Update(cmd())
	got = updated.(Model)
	if len(copied) != 2 || copied[1] != "Authorization: Bearer secret" || got.status != "Copied remote auth header" {
		t.Fatalf("auth copy copied=%#v status=%q", copied, got.status)
	}
	if strings.Contains(got.View().Content, "Bearer secret") {
		t.Fatalf("remote dialog leaked full token:\n%s", got.View().Content)
	}
}
