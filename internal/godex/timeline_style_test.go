package godex

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestDefaultTimelineStyleIsMinimal(t *testing.T) {
	if got := DefaultConfig().TimelineStyle; got != TimelineStyleMinimal {
		t.Fatalf("default timeline style = %q, want %q", got, TimelineStyleMinimal)
	}
	if got := NormalizeTimelineStyle(""); got != TimelineStyleMinimal {
		t.Fatalf("empty timeline style = %q, want %q", got, TimelineStyleMinimal)
	}
}

func TestSettingsRoundTripTimelineStyle(t *testing.T) {
	dataDir := t.TempDir()

	if err := SaveSettings(dataDir, Settings{TimelineStyle: TimelineStyleMinimal}); err != nil {
		t.Fatalf("save settings: %v", err)
	}
	settings, err := LoadSettings(dataDir)
	if err != nil {
		t.Fatalf("load settings: %v", err)
	}
	if settings.TimelineStyle != TimelineStyleMinimal {
		t.Fatalf("timeline style = %q, want %q", settings.TimelineStyle, TimelineStyleMinimal)
	}
	data, err := os.ReadFile(filepath.Join(dataDir, "config.toml"))
	if err != nil {
		t.Fatalf("read config.toml: %v", err)
	}
	if !strings.Contains(string(data), `timeline_style = "minimal"`) {
		t.Fatalf("config.toml missing timeline_style:\n%s", string(data))
	}
}
