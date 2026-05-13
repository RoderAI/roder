package godex

import (
	"testing"
)

func TestBuildProviderAnthropicUsesAnthropicConfig(t *testing.T) {
	t.Setenv("OPENAI_API_KEY", "")
	t.Setenv("ANTHROPIC_API_KEY", "test-anthropic-key")

	prov, err := buildProvider(Config{Provider: ProviderAnthropic, Model: "claude-sonnet-4-6"})
	if err != nil {
		t.Fatalf("build provider: %v", err)
	}
	if prov.Name() != ProviderAnthropic {
		t.Fatalf("provider name = %q", prov.Name())
	}
}

func TestBuildProviderAnthropicDoesNotRequireOpenAIKey(t *testing.T) {
	t.Setenv("OPENAI_API_KEY", "")
	t.Setenv("ANTHROPIC_API_KEY", "")

	prov, err := buildProvider(Config{Provider: ProviderAnthropic, Model: "claude-haiku-4-5-20251001"})
	if err != nil {
		t.Fatalf("build provider without keys: %v", err)
	}
	if prov.Name() != ProviderAnthropic {
		t.Fatalf("provider name = %q", prov.Name())
	}
}
