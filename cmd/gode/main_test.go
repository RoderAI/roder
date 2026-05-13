package main

import (
	"path/filepath"
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
