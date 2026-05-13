package configstore

import (
	"path/filepath"
	"testing"

	"github.com/pandelisz/gode/internal/godex"
)

func TestLoadDefaultsTimelineStyleToMinimal(t *testing.T) {
	loaded, err := Load(LoadOptions{DataDir: t.TempDir(), Env: []string{"HOME=" + t.TempDir()}})
	if err != nil {
		t.Fatalf("load: %v", err)
	}
	if loaded.Config.TimelineStyle != godex.TimelineStyleMinimal {
		t.Fatalf("timeline style = %q, want %q", loaded.Config.TimelineStyle, godex.TimelineStyleMinimal)
	}
}

func TestLoadReadsTimelineStyleFromConfig(t *testing.T) {
	dataDir := t.TempDir()
	writeFile(t, filepath.Join(dataDir, "config.toml"), `timeline_style = "minimal"`)

	loaded, err := Load(LoadOptions{DataDir: dataDir, Env: []string{"HOME=" + t.TempDir()}})
	if err != nil {
		t.Fatalf("load: %v", err)
	}
	if loaded.Config.TimelineStyle != godex.TimelineStyleMinimal {
		t.Fatalf("timeline style = %q, want %q", loaded.Config.TimelineStyle, godex.TimelineStyleMinimal)
	}
}
