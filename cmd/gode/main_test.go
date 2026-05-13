package main

import "testing"

func TestParseConfigAppliesFlags(t *testing.T) {
	cfg, err := parseConfig([]string{
		"--workspace", "/tmp/workspace",
		"--data-dir", "/tmp/data",
		"--provider", "mock",
		"--model", "test-model",
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
	if !cfg.AutoApprove {
		t.Fatal("auto approve = false")
	}
}
