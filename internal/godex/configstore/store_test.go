package configstore

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/pandelisz/gode/internal/godex"
)

func TestLoadMergesSourcesInOrder(t *testing.T) {
	root := t.TempDir()
	workspace := filepath.Join(root, "repo", "nested")
	dataDir := filepath.Join(root, "data")
	xdgDir := filepath.Join(root, "xdg")
	mustMkdir(t, workspace)
	mustMkdir(t, filepath.Join(xdgDir, "gode"))
	mustMkdir(t, dataDir)

	writeFile(t, filepath.Join(xdgDir, "gode", "config.json"), `{
		"provider": "mock",
		"model": "global-model",
		"reasoning": "none",
		"telemetry_endpoint": "global:4317",
		"context_paths": ["GLOBAL.md"]
	}`)
	writeFile(t, filepath.Join(root, "repo", ".gode.toml"), `
provider = "openai"
model = "project-model"
reasoning = "medium"
auto_approve = false
context_paths = ["AGENTS.md"]
disabled_tools = ["shell"]
`)
	writeFile(t, filepath.Join(dataDir, "config.toml"), `
default_model = "data-model"
default_reasoning = "high"
fast_mode = true
`)

	flags := godex.Config{Model: "flag-model", AutoApprove: true}
	loaded, err := Load(LoadOptions{
		Workspace: workspace,
		DataDir:   dataDir,
		Env: []string{
			"XDG_CONFIG_HOME=" + xdgDir,
			"HOME=" + filepath.Join(root, "home"),
			"GODE_PROVIDER=openai",
			"GODE_MODEL=env-model",
			"GODE_REASONING=low",
			"GODE_AUTO_APPROVE=false",
		},
		Flags:   flags,
		FlagSet: map[string]bool{"model": true, "auto-approve": true},
	})
	if err != nil {
		t.Fatalf("load: %v", err)
	}

	if loaded.Config.Workspace != workspace {
		t.Fatalf("workspace = %q", loaded.Config.Workspace)
	}
	if loaded.Config.DataDir != dataDir {
		t.Fatalf("data dir = %q", loaded.Config.DataDir)
	}
	if loaded.Config.Provider != "openai" {
		t.Fatalf("provider = %q", loaded.Config.Provider)
	}
	if loaded.Config.Model != "flag-model" {
		t.Fatalf("model = %q", loaded.Config.Model)
	}
	if loaded.Config.Reasoning != "low" {
		t.Fatalf("reasoning = %q", loaded.Config.Reasoning)
	}
	if !loaded.Config.AutoApprove {
		t.Fatal("auto approve should come from explicit flag")
	}
	if !loaded.Config.FastMode {
		t.Fatal("fast mode should come from data config")
	}
	if loaded.Config.TelemetryEndpoint != "global:4317" {
		t.Fatalf("telemetry endpoint = %q", loaded.Config.TelemetryEndpoint)
	}
	if got := strings.Join(loaded.Config.ContextPaths, ","); got != "AGENTS.md" {
		t.Fatalf("context paths = %q", got)
	}
	if got := strings.Join(loaded.Config.DisabledTools, ","); got != "shell" {
		t.Fatalf("disabled tools = %q", got)
	}
	if len(loaded.Paths) != 3 {
		t.Fatalf("paths = %#v", loaded.Paths)
	}
	for _, want := range []string{
		filepath.Join(xdgDir, "gode", "config.json"),
		filepath.Join(root, "repo", ".gode.toml"),
		filepath.Join(dataDir, "config.toml"),
	} {
		if !containsPath(loaded.Paths, want) {
			t.Fatalf("paths missing %q: %#v", want, loaded.Paths)
		}
	}
}

func TestLoadEmptyConfigKeepsDefaults(t *testing.T) {
	defaults := godex.DefaultConfig()
	dataDir := filepath.Join(t.TempDir(), "data")
	loaded, err := Load(LoadOptions{DataDir: dataDir, Env: []string{"HOME=" + t.TempDir()}})
	if err != nil {
		t.Fatalf("load: %v", err)
	}
	if loaded.Config.Workspace != defaults.Workspace {
		t.Fatalf("workspace = %q, want %q", loaded.Config.Workspace, defaults.Workspace)
	}
	if loaded.Config.DataDir != dataDir {
		t.Fatalf("data dir = %q, want %q", loaded.Config.DataDir, dataDir)
	}
	if loaded.Config.Provider != defaults.Provider {
		t.Fatalf("provider = %q, want %q", loaded.Config.Provider, defaults.Provider)
	}
	if loaded.Config.Model != defaults.Model {
		t.Fatalf("model = %q, want %q", loaded.Config.Model, defaults.Model)
	}
	if loaded.Config.Reasoning != defaults.Reasoning {
		t.Fatalf("reasoning = %q, want %q", loaded.Config.Reasoning, defaults.Reasoning)
	}
	if loaded.Config.TelemetryEndpoint != defaults.TelemetryEndpoint {
		t.Fatalf("telemetry endpoint = %q, want %q", loaded.Config.TelemetryEndpoint, defaults.TelemetryEndpoint)
	}
	if len(loaded.Paths) != 0 {
		t.Fatalf("paths = %#v", loaded.Paths)
	}
}

func TestLoadParseErrorIdentifiesPathAndSource(t *testing.T) {
	root := t.TempDir()
	workspace := filepath.Join(root, "repo")
	mustMkdir(t, workspace)
	path := filepath.Join(workspace, ".gode.toml")
	writeFile(t, path, "model =")

	_, err := Load(LoadOptions{Workspace: workspace, Env: []string{"HOME=" + filepath.Join(root, "home")}})
	if err == nil {
		t.Fatal("expected parse error")
	}
	for _, want := range []string{"parse project config", path} {
		if !strings.Contains(err.Error(), want) {
			t.Fatalf("error should contain %q, got %v", want, err)
		}
	}
}

func mustMkdir(t *testing.T, path string) {
	t.Helper()
	if err := os.MkdirAll(path, 0o700); err != nil {
		t.Fatalf("mkdir %s: %v", path, err)
	}
}

func writeFile(t *testing.T, path string, contents string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		t.Fatalf("mkdir %s: %v", filepath.Dir(path), err)
	}
	if err := os.WriteFile(path, []byte(contents), 0o600); err != nil {
		t.Fatalf("write %s: %v", path, err)
	}
}

func containsPath(paths []string, want string) bool {
	for _, path := range paths {
		if path == want {
			return true
		}
	}
	return false
}
