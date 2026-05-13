package main

import (
	"testing"

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
