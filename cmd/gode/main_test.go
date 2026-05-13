package main

import (
	"io"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/pandelisz/gode/internal/godex"
	"github.com/pandelisz/gode/internal/godex/appserver"
)

func TestParseConfigAppliesFlags(t *testing.T) {
	cfg, err := parseConfig([]string{
		"--workspace", "/tmp/workspace",
		"--data-dir", "/tmp/data",
		"--provider", "mock",
		"--model", "test-model",
		"--reasoning", "low",
		"--auto-approve",
	})
	if err != nil {
		t.Fatalf("parse: %v", err)
	}
	if cfg.Workspace != "/tmp/workspace" {
		t.Fatalf("workspace = %q", cfg.Workspace)
	}
	if cfg.DataDir != "/tmp/data" {
		t.Fatalf("data dir = %q", cfg.DataDir)
	}
	if cfg.Provider != "mock" {
		t.Fatalf("provider = %q", cfg.Provider)
	}
	if cfg.Model != "test-model" {
		t.Fatalf("model = %q", cfg.Model)
	}
	if cfg.Reasoning != "low" {
		t.Fatalf("reasoning = %q", cfg.Reasoning)
	}
	if !cfg.AutoApprove {
		t.Fatal("auto approve = false")
	}
}

func TestParseAppServerConfigAppliesFlags(t *testing.T) {
	cfg, listen, err := parseAppServerConfig([]string{
		"--listen", "ws://127.0.0.1:0",
		"--workspace", "/tmp/workspace",
		"--data-dir", "/tmp/data",
		"--provider", "mock",
		"--model", "test-model",
		"--reasoning", "low",
		"--auto-approve",
	})
	if err != nil {
		t.Fatalf("parse: %v", err)
	}
	if listen.Kind != appserver.TransportWebSocket {
		t.Fatalf("listen kind = %v", listen.Kind)
	}
	if listen.Address != "127.0.0.1:0" {
		t.Fatalf("listen address = %q", listen.Address)
	}
	if cfg.Workspace != "/tmp/workspace" {
		t.Fatalf("workspace = %q", cfg.Workspace)
	}
	if cfg.DataDir != "/tmp/data" {
		t.Fatalf("data dir = %q", cfg.DataDir)
	}
	if cfg.Provider != "mock" {
		t.Fatalf("provider = %q", cfg.Provider)
	}
	if cfg.Model != "test-model" {
		t.Fatalf("model = %q", cfg.Model)
	}
	if cfg.Reasoning != "low" {
		t.Fatalf("reasoning = %q", cfg.Reasoning)
	}
	if !cfg.AutoApprove {
		t.Fatal("auto approve = false")
	}
}

func TestParseConfigUsesSavedDefaultModel(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	dataDir := filepath.Join(home, ".gode")
	if err := godex.SaveSettings(dataDir, godex.Settings{DefaultModel: "gpt-5.4-mini", DefaultReasoning: godex.ReasoningHigh, FastMode: true}); err != nil {
		t.Fatalf("save settings: %v", err)
	}

	cfg, err := parseConfig(nil)
	if err != nil {
		t.Fatalf("parse: %v", err)
	}
	if cfg.Model != "gpt-5.4-mini" {
		t.Fatalf("model = %q", cfg.Model)
	}
	if cfg.Provider != godex.ProviderOpenAI {
		t.Fatalf("provider = %q", cfg.Provider)
	}
	if cfg.Reasoning != godex.ReasoningHigh {
		t.Fatalf("reasoning = %q", cfg.Reasoning)
	}
	if !cfg.FastMode {
		t.Fatal("fast mode = false")
	}
}

func TestParseConfigUsesLegacySettingsJSON(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	dataDir := filepath.Join(home, ".gode")
	if err := os.MkdirAll(dataDir, 0o700); err != nil {
		t.Fatalf("mkdir data dir: %v", err)
	}
	if err := os.WriteFile(filepath.Join(dataDir, "settings.json"), []byte(`{"default_model":"gpt-5.4-mini","default_reasoning":"high"}`), 0o600); err != nil {
		t.Fatalf("write settings.json: %v", err)
	}

	cfg, err := parseConfig(nil)
	if err != nil {
		t.Fatalf("parse: %v", err)
	}
	if cfg.Model != "gpt-5.4-mini" {
		t.Fatalf("model = %q", cfg.Model)
	}
	if cfg.Reasoning != godex.ReasoningHigh {
		t.Fatalf("reasoning = %q", cfg.Reasoning)
	}
}

func TestParseConfigModelFlagOverridesSavedDefaultModel(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	dataDir := filepath.Join(home, ".gode")
	if err := godex.SaveSettings(dataDir, godex.Settings{DefaultModel: "gpt-5.4-mini"}); err != nil {
		t.Fatalf("save settings: %v", err)
	}

	cfg, err := parseConfig([]string{"--model", "gpt-flag"})
	if err != nil {
		t.Fatalf("parse: %v", err)
	}
	if cfg.Model != "gpt-flag" {
		t.Fatalf("model = %q", cfg.Model)
	}
}

func TestParseConfigProviderFlagOverridesSavedDefaultModelProvider(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	dataDir := filepath.Join(home, ".gode")
	if err := godex.SaveSettings(dataDir, godex.Settings{DefaultModel: "gpt-5.4-mini"}); err != nil {
		t.Fatalf("save settings: %v", err)
	}

	cfg, err := parseConfig([]string{"--provider", "mock"})
	if err != nil {
		t.Fatalf("parse: %v", err)
	}
	if cfg.Model != "gpt-5.4-mini" {
		t.Fatalf("model = %q", cfg.Model)
	}
	if cfg.Provider != "mock" {
		t.Fatalf("provider = %q", cfg.Provider)
	}
}

func TestParseConfigUsesProjectConfig(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	workspace := filepath.Join(t.TempDir(), "repo", "nested")
	if err := os.MkdirAll(workspace, 0o700); err != nil {
		t.Fatalf("mkdir workspace: %v", err)
	}
	projectConfig := filepath.Join(filepath.Dir(workspace), ".gode.toml")
	if err := os.WriteFile(projectConfig, []byte(`provider = "mock"
model = "mock"
reasoning = "none"
auto_approve = true
`), 0o600); err != nil {
		t.Fatalf("write project config: %v", err)
	}

	cfg, err := parseConfig([]string{"--workspace", workspace})
	if err != nil {
		t.Fatalf("parse: %v", err)
	}
	if cfg.Provider != "mock" || cfg.Model != "mock" || cfg.Reasoning != "none" || !cfg.AutoApprove {
		t.Fatalf("cfg = %#v", cfg)
	}
}

func TestRunDirsPrintsConfigPaths(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	workspace := filepath.Join(t.TempDir(), "repo")
	dataDir := filepath.Join(t.TempDir(), "data")
	if err := os.MkdirAll(workspace, 0o700); err != nil {
		t.Fatalf("mkdir workspace: %v", err)
	}
	if err := godex.SaveSettings(dataDir, godex.Settings{DefaultModel: "mock"}); err != nil {
		t.Fatalf("save settings: %v", err)
	}

	out := captureStdout(t, func() error {
		return runDirs([]string{"--workspace", workspace, "--data-dir", dataDir})
	})
	for _, want := range []string{"workspace\t" + workspace, "data_dir\t" + dataDir, filepath.Join(dataDir, "config.toml")} {
		if !strings.Contains(out, want) {
			t.Fatalf("dirs output missing %q:\n%s", want, out)
		}
	}
}

func TestRunModelsPrintsLocalCatalog(t *testing.T) {
	t.Setenv("HOME", t.TempDir())
	out := captureStdout(t, func() error {
		return runModels(nil)
	})
	for _, want := range []string{"openai\tgpt-5.4-mini", "anthropic-compatible\tclaude-sonnet-4.5"} {
		if !strings.Contains(out, want) {
			t.Fatalf("models output missing %q:\n%s", want, out)
		}
	}
}

func TestRunConfigSchemaPrintsJSON(t *testing.T) {
	out := captureStdout(t, func() error {
		return runConfig([]string{"schema"})
	})
	for _, want := range []string{`"provider"`, `"context_paths"`, `"selected_models"`} {
		if !strings.Contains(out, want) {
			t.Fatalf("schema output missing %q:\n%s", want, out)
		}
	}
}

func captureStdout(t *testing.T, fn func() error) string {
	t.Helper()
	old := os.Stdout
	read, write, err := os.Pipe()
	if err != nil {
		t.Fatalf("pipe: %v", err)
	}
	os.Stdout = write
	runErr := fn()
	if err := write.Close(); err != nil {
		t.Fatalf("close pipe writer: %v", err)
	}
	os.Stdout = old
	data, err := io.ReadAll(read)
	if err != nil {
		t.Fatalf("read stdout: %v", err)
	}
	if runErr != nil {
		t.Fatalf("run: %v\nstdout:\n%s", runErr, data)
	}
	return string(data)
}
