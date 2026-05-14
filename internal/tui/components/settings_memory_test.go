package components

import (
	"strings"
	"testing"

	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

func TestSettingsDialogRendersMemoryRows(t *testing.T) {
	dialog := viewmodel.SettingsDialog{
		Title:  "Memories",
		Screen: viewmodel.SettingsScreenMemories,
		Memory: viewmodel.SettingsMemoryState{
			Rows: []viewmodel.SettingsMemoryRow{
				{ID: "enabled", Label: "Enabled", Value: "on", Selected: true},
				{ID: "count", Label: "Stored memories", Value: "2"},
				{ID: "database", Label: "Database", Value: "/tmp/memories.sqlite3"},
			},
		},
	}

	view := SettingsDialogBox(100, dialog, nil)
	for _, want := range []string{"Memories", "Enabled", "on", "Stored memories", "2", "/tmp/memories.sqlite3"} {
		if !strings.Contains(view, want) {
			t.Fatalf("memory settings view missing %q:\n%s", want, view)
		}
	}
}
