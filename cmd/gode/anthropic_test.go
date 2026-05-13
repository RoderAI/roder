package main

import (
	"strings"
	"testing"
)

func TestParseConfigAnthropicProviderAndModel(t *testing.T) {
	cfg, err := parseConfig([]string{"--provider", "anthropic", "--model", "claude-sonnet-4-6"})
	if err != nil {
		t.Fatalf("parse: %v", err)
	}
	if cfg.Provider != "anthropic" || cfg.Model != "claude-sonnet-4-6" {
		t.Fatalf("provider/model = %q/%q", cfg.Provider, cfg.Model)
	}
}

func TestModelsOutputIncludesAnthropicModels(t *testing.T) {
	out := captureStdout(t, func() error {
		return runModels(nil)
	})
	for _, want := range []string{"anthropic\tclaude-sonnet-4-6", "anthropic\tclaude-opus-4-7", "anthropic\tclaude-haiku-4-5-20251001"} {
		if !strings.Contains(out, want) {
			t.Fatalf("models output missing %q:\n%s", want, out)
		}
	}
}
