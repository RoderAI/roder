package godex

import (
	"testing"

	"github.com/pandelisz/gode/internal/godex/codexauth"
)

func TestBuiltInModelCatalogHasDefaultModelConfig(t *testing.T) {
	model, ok := LookupModel(DefaultModelID)
	if !ok {
		t.Fatalf("default model %q missing from catalog", DefaultModelID)
	}
	if model.Provider != ProviderOpenAI {
		t.Fatalf("default model provider = %q", model.Provider)
	}
	if model.DefaultReasoning != ReasoningMedium {
		t.Fatalf("default reasoning = %q", model.DefaultReasoning)
	}
	if !model.SupportsReasoning(ReasoningXHigh) {
		t.Fatalf("default model should support %q", ReasoningXHigh)
	}
	if model.ContextWindow <= 0 {
		t.Fatalf("context window = %d", model.ContextWindow)
	}
}

func TestBuiltInModelCatalogReturnsCopies(t *testing.T) {
	models := BuiltInModels(false)
	if len(models) == 0 {
		t.Fatal("built-in model catalog is empty")
	}
	models[0].ID = "mutated"

	model, ok := LookupModel(DefaultModelID)
	if !ok {
		t.Fatalf("default model %q missing from catalog", DefaultModelID)
	}
	if model.ID != DefaultModelID {
		t.Fatalf("catalog was mutated, got default model id %q", model.ID)
	}
}

func TestConfigDefaultsComeFromDefaultModelConfig(t *testing.T) {
	cfg := DefaultConfig()
	if cfg.Model != DefaultModelID {
		t.Fatalf("model = %q", cfg.Model)
	}
	if cfg.Provider != ProviderOpenAI {
		t.Fatalf("provider = %q", cfg.Provider)
	}
	if cfg.Reasoning != ReasoningMedium {
		t.Fatalf("reasoning = %q", cfg.Reasoning)
	}
}

func TestConfigWithKnownModelAppliesModelProviderAndReasoningDefaults(t *testing.T) {
	cfg := (Config{Model: "gpt-5.4-mini"}).withDefaults()
	if cfg.Provider != ProviderOpenAI {
		t.Fatalf("provider = %q", cfg.Provider)
	}
	if cfg.Reasoning != ReasoningMedium {
		t.Fatalf("reasoning = %q", cfg.Reasoning)
	}
}

func TestConfigWithExplicitProviderKeepsProvider(t *testing.T) {
	cfg := (Config{Model: "gpt-5.4-mini", Provider: ProviderMock}).withDefaults()
	if cfg.Provider != ProviderMock {
		t.Fatalf("provider = %q", cfg.Provider)
	}
	if cfg.Reasoning != ReasoningMedium {
		t.Fatalf("reasoning = %q", cfg.Reasoning)
	}
}

func TestConfigWithExplicitProviderAndNoModelUsesProviderDefaultModel(t *testing.T) {
	cfg := (Config{Provider: ProviderMock}).withDefaults()
	if cfg.Model != "mock" {
		t.Fatalf("model = %q", cfg.Model)
	}
	if cfg.Reasoning != ReasoningNone {
		t.Fatalf("reasoning = %q", cfg.Reasoning)
	}
}

func TestDisplayProviderShowsCodexWhenOpenAIGPTUsesCodexAuth(t *testing.T) {
	dataDir := t.TempDir()
	cfg := Config{
		DataDir:   dataDir,
		Workspace: t.TempDir(),
		Provider:  ProviderOpenAI,
		Model:     DefaultModelID,
		Reasoning: ReasoningMedium,
	}

	if got := DisplayProvider(cfg); got != ProviderOpenAI {
		t.Fatalf("display provider before sign-in = %q", got)
	}
	if err := (codexauth.Store{DataDir: dataDir}).Save(codexauth.Tokens{Refresh: "refresh"}); err != nil {
		t.Fatalf("save codex auth: %v", err)
	}
	if !UsesCodexAuth(cfg) {
		t.Fatal("expected codex auth for signed-in OpenAI GPT model")
	}
	if got := DisplayProvider(cfg); got != ProviderCodex {
		t.Fatalf("display provider after sign-in = %q", got)
	}
	if got := DisplayModelLabel(cfg); got != ProviderCodex+"/"+DefaultModelID {
		t.Fatalf("display model label = %q", got)
	}
}

func TestDisplayProviderKeepsOpenAIForNonGPTModels(t *testing.T) {
	dataDir := t.TempDir()
	if err := (codexauth.Store{DataDir: dataDir}).Save(codexauth.Tokens{Refresh: "refresh"}); err != nil {
		t.Fatalf("save codex auth: %v", err)
	}
	cfg := Config{
		DataDir:   dataDir,
		Workspace: t.TempDir(),
		Provider:  ProviderOpenAI,
		Model:     "text-embedding-3-small",
		Reasoning: ReasoningNone,
	}

	if UsesCodexAuth(cfg) {
		t.Fatal("non-GPT model should not use codex auth")
	}
	if got := DisplayProvider(cfg); got != ProviderOpenAI {
		t.Fatalf("display provider = %q", got)
	}
}
