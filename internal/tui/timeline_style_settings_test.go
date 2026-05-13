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

func TestSettingsTimelineStyleToggleSavesDefault(t *testing.T) {
	dataDir := t.TempDir()
	app, err := godex.New(context.Background(), godex.Config{
		DataDir:       dataDir,
		Workspace:     filepath.Join(t.TempDir(), "workspace"),
		Provider:      "mock",
		TimelineStyle: godex.TimelineStyleDetailed,
	})
	if err != nil {
		t.Fatalf("app: %v", err)
	}
	defer app.Close(context.Background())

	model := New(app)
	model.openSettings()
	model.settings.MenuIndex = settingsMenuIndex(t, model.settings, "timeline-style")
	updated, _ := model.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	got := updated.(Model)

	if !got.settings.Open {
		t.Fatal("settings dialog should remain open after toggling timeline style")
	}
	if app.Config.TimelineStyle != godex.TimelineStyleMinimal {
		t.Fatalf("app timeline style = %q, want %q", app.Config.TimelineStyle, godex.TimelineStyleMinimal)
	}
	settings, err := godex.LoadSettings(dataDir)
	if err != nil {
		t.Fatalf("load settings: %v", err)
	}
	if settings.TimelineStyle != godex.TimelineStyleMinimal {
		t.Fatalf("saved timeline style = %q, want %q", settings.TimelineStyle, godex.TimelineStyleMinimal)
	}
	data, err := os.ReadFile(filepath.Join(dataDir, "config.toml"))
	if err != nil {
		t.Fatalf("read config.toml: %v", err)
	}
	if !strings.Contains(string(data), `timeline_style = "minimal"`) {
		t.Fatalf("config.toml missing timeline_style:\n%s", string(data))
	}
	if view := got.View().Content; !strings.Contains(view, "Timeline Style") || !strings.Contains(view, "minimal") {
		t.Fatalf("settings view missing timeline style:\n%s", view)
	}
}
