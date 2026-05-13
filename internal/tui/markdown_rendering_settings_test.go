package tui

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"

	"github.com/pandelisz/gode/internal/godex"
)

func TestSettingsToggleMarkdownRenderingPersists(t *testing.T) {
	dataDir := t.TempDir()
	app, err := godex.New(context.Background(), godex.Config{
		DataDir:           dataDir,
		Workspace:         filepath.Join(t.TempDir(), "workspace"),
		Provider:          "mock",
		MarkdownRendering: false,
	})
	if err != nil {
		t.Fatalf("app: %v", err)
	}
	defer app.Close(context.Background())

	model := New(app)
	model.width = 100
	model.height = 30

	model.openSettings()
	model.settings.MenuIndex = settingsMenuIndex(t, model.settings, "markdown-rendering")

	updated, _ := model.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	model = updated.(Model)

	if !model.app.Config.MarkdownRendering {
		t.Fatal("expected markdown rendering enabled in app config")
	}
	loaded, err := godex.LoadSettings(dataDir)
	if err != nil {
		t.Fatalf("load settings: %v", err)
	}
	if !loaded.MarkdownRendering {
		t.Fatal("expected markdown rendering enabled in persisted settings")
	}
	raw, err := os.ReadFile(filepath.Join(dataDir, "config.toml"))
	if err != nil {
		t.Fatalf("read config.toml: %v", err)
	}
	if !strings.Contains(string(raw), "markdown_rendering = true") {
		t.Fatalf("expected markdown_rendering in config.toml, got:\n%s", string(raw))
	}

	view := model.View().Content
	if !strings.Contains(view, "Markdown Rendering") || !strings.Contains(view, "on") {
		t.Fatalf("expected settings view to show markdown rendering on, got:\n%s", view)
	}
}
