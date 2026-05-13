package tui

import (
	"context"
	"path/filepath"
	"testing"

	tea "charm.land/bubbletea/v2"
	"github.com/pandelisz/gode/internal/godex"
)

func TestPermissionModeControlsComposerBorder(t *testing.T) {
	app, err := godex.New(context.Background(), godex.Config{
		DataDir:     t.TempDir(),
		Workspace:   filepath.Join(t.TempDir(), "workspace"),
		Provider:    "mock",
		AutoApprove: false,
	})
	if err != nil {
		t.Fatalf("app: %v", err)
	}
	defer app.Close(context.Background())

	model := New(app)
	model.width = 80
	model.height = 24

	if model.viewModel().AutoApprove {
		t.Fatal("normal mode should not mark composer as auto approve")
	}

	updated, _ := model.Update(tea.KeyPressMsg{Code: tea.KeyTab, Mod: tea.ModShift})
	got := updated.(Model)
	if !got.viewModel().AutoApprove {
		t.Fatal("allow-all mode should mark composer as auto approve")
	}
}
