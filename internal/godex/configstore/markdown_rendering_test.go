package configstore

import (
	"os"
	"path/filepath"
	"testing"
)

func TestLoadMarkdownRenderingFromConfigFile(t *testing.T) {
	dataDir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dataDir, "config.toml"), []byte("markdown_rendering = true\n"), 0o600); err != nil {
		t.Fatalf("write config.toml: %v", err)
	}

	loaded, err := Load(LoadOptions{DataDir: dataDir})
	if err != nil {
		t.Fatalf("load config: %v", err)
	}
	if !loaded.Config.MarkdownRendering {
		t.Fatal("expected markdown rendering to be enabled from config.toml")
	}
}
