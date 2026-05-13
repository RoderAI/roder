package godex

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestSettingsRoundTripDefaultModel(t *testing.T) {
	dataDir := t.TempDir()

	if err := SaveSettings(dataDir, Settings{DefaultModel: "gpt-5.5", DefaultReasoning: ReasoningHigh, FastMode: true}); err != nil {
		t.Fatalf("save settings: %v", err)
	}

	settings, err := LoadSettings(dataDir)
	if err != nil {
		t.Fatalf("load settings: %v", err)
	}
	if settings.DefaultModel != "gpt-5.5" {
		t.Fatalf("default model = %q", settings.DefaultModel)
	}
	if settings.DefaultReasoning != ReasoningHigh {
		t.Fatalf("default reasoning = %q", settings.DefaultReasoning)
	}
	if !settings.FastMode {
		t.Fatal("fast mode = false")
	}
	if _, err := os.Stat(filepath.Join(dataDir, "config.toml")); err != nil {
		t.Fatalf("config.toml should be written: %v", err)
	}
	if _, err := os.Stat(filepath.Join(dataDir, "settings.json")); !os.IsNotExist(err) {
		t.Fatalf("settings.json should not be written, stat err = %v", err)
	}
	data, err := os.ReadFile(filepath.Join(dataDir, "config.toml"))
	if err != nil {
		t.Fatalf("read config.toml: %v", err)
	}
	for _, want := range []string{`default_model = "gpt-5.5"`, `default_reasoning = "high"`, `fast_mode = true`} {
		if !strings.Contains(string(data), want) {
			t.Fatalf("config.toml should contain %q, got:\n%s", want, string(data))
		}
	}
	if strings.Contains(string(data), "{") {
		t.Fatalf("config.toml should contain TOML default model, got:\n%s", string(data))
	}

	if _, err := LoadSettings(filepath.Join(dataDir, "missing")); err != nil {
		t.Fatalf("missing settings should load empty defaults: %v", err)
	}
}

func TestDefaultDataDirUsesWindowsConfigDir(t *testing.T) {
	if got := defaultDataDirFor("windows", "C:/Users/pz", "C:/Users/pz/AppData/Roaming"); got != filepath.Join("C:/Users/pz/AppData/Roaming", "gode") {
		t.Fatalf("windows data dir = %q", got)
	}
	if got := defaultDataDirFor("linux", "/home/pz", "/tmp/config"); got != filepath.Join("/home/pz", ".gode") {
		t.Fatalf("linux data dir = %q", got)
	}
	if got := defaultDataDirFor("darwin", "/Users/pz", "/tmp/config"); got != filepath.Join("/Users/pz", ".gode") {
		t.Fatalf("darwin data dir = %q", got)
	}
}
