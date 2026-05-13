package godex

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestSettingsPersistMarkdownRendering(t *testing.T) {
	dataDir := t.TempDir()

	if err := SaveSettings(dataDir, Settings{MarkdownRendering: true}); err != nil {
		t.Fatalf("save settings: %v", err)
	}

	loaded, err := LoadSettings(dataDir)
	if err != nil {
		t.Fatalf("load settings: %v", err)
	}
	if !loaded.MarkdownRendering {
		t.Fatal("expected markdown rendering to be enabled after reload")
	}

	raw, err := os.ReadFile(filepath.Join(dataDir, "config.toml"))
	if err != nil {
		t.Fatalf("read config.toml: %v", err)
	}
	if !strings.Contains(string(raw), "markdown_rendering = true") {
		t.Fatalf("expected markdown_rendering in config.toml, got:\n%s", string(raw))
	}
}
