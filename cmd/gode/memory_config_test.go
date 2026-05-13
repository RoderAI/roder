package main

import (
	"path/filepath"
	"testing"

	"github.com/pandelisz/gode/internal/godex"
	"github.com/pandelisz/gode/internal/godex/memory"
)

func TestParseConfigModelAndReasoningFlagsPreserveMemorySettings(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	dataDir := filepath.Join(home, ".gode")
	if err := godex.SaveSettings(dataDir, godex.Settings{
		DefaultModel:     "gpt-5.4-mini",
		DefaultReasoning: godex.ReasoningHigh,
		Memories: memory.Settings{
			Enabled:        boolPtr(false),
			AutoRecall:     boolPtr(false),
			AutoObserve:    boolPtr(true),
			EmbeddingModel: "custom-embedding",
			RecallLimit:    9,
			DatabasePath:   filepath.Join(home, "custom.sqlite3"),
		},
	}); err != nil {
		t.Fatalf("save settings: %v", err)
	}

	cfg, err := parseConfig([]string{"--model", "gpt-flag", "--reasoning", "low"})
	if err != nil {
		t.Fatalf("parse: %v", err)
	}
	if cfg.Model != "gpt-flag" || cfg.Reasoning != godex.ReasoningLow {
		t.Fatalf("model/reasoning = %q/%q", cfg.Model, cfg.Reasoning)
	}
	if cfg.Memories.Enabled || cfg.Memories.AutoRecall || !cfg.Memories.AutoObserve {
		t.Fatalf("memory bools = %#v", cfg.Memories)
	}
	if cfg.Memories.EmbeddingModel != "custom-embedding" || cfg.Memories.RecallLimit != 9 || cfg.Memories.DatabasePath != filepath.Join(home, "custom.sqlite3") {
		t.Fatalf("memory settings = %#v", cfg.Memories)
	}
}

func boolPtr(value bool) *bool {
	return &value
}
