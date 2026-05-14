package godex

import (
	"strings"
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
	if model.AutoCompactTokenLimit <= 0 {
		t.Fatalf("auto compact limit = %d", model.AutoCompactTokenLimit)
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

func TestGPT55ModelConfigUsesOneMillionClassWindow(t *testing.T) {
	model := ModelConfigFor("gpt-5.5")
	if model.ContextWindow != 1050000 {
		t.Fatalf("context window = %d", model.ContextWindow)
	}
	if model.MaxContextWindow != 1050000 {
		t.Fatalf("max context window = %d", model.MaxContextWindow)
	}
	if model.AutoCompactTokenLimit != 800000 {
		t.Fatalf("auto compact limit = %d", model.AutoCompactTokenLimit)
	}
	if !model.SupportsCompaction {
		t.Fatal("gpt-5.5 should support compaction")
	}
}

func TestVisibleOpenAIModelsHaveContextWindowsAndThresholds(t *testing.T) {
	for _, model := range BuiltInModels(false) {
		if model.Provider != ProviderOpenAI {
			continue
		}
		if model.ContextWindow <= 0 {
			t.Fatalf("%s context window = %d", model.ID, model.ContextWindow)
		}
		if model.AutoCompactTokenLimit <= 0 {
			t.Fatalf("%s auto compact limit = %d", model.ID, model.AutoCompactTokenLimit)
		}
	}
}

func TestFallbackModelDoesNotClaimOneMillionClassWindow(t *testing.T) {
	model := ModelConfigFor("gpt-future")
	if model.ContextWindow != 272000 {
		t.Fatalf("context window = %d", model.ContextWindow)
	}
	if model.AutoCompactTokenLimit != 217600 {
		t.Fatalf("auto compact limit = %d", model.AutoCompactTokenLimit)
	}
	if model.SupportsCompaction {
		t.Fatal("fallback model should not claim compaction support")
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

func TestAnthropicProviderAndModels(t *testing.T) {
	provider, ok := LookupProvider(ProviderAnthropic)
	if !ok {
		t.Fatal("anthropic provider missing")
	}
	if provider.Kind != ProviderKindAnthropic || provider.EnvKey != "ANTHROPIC_API_KEY" || provider.DefaultModel != "claude-sonnet-4-6" {
		t.Fatalf("anthropic provider = %#v", provider)
	}
	opus := ModelConfigFor("claude-opus-4-7")
	if opus.Provider != ProviderAnthropic || opus.ContextWindow != 1000000 || opus.MaxContextWindow != 1000000 || opus.DefaultReasoning != ReasoningHigh {
		t.Fatalf("opus config = %#v", opus)
	}
	haiku := ModelConfigFor("claude-haiku-4-5-20251001")
	if haiku.ContextWindow != 200000 || haiku.DefaultReasoning != ReasoningLow {
		t.Fatalf("haiku config = %#v", haiku)
	}
}

func TestAnthropicSonnetAndOpusModelsUseMillionTokenContext(t *testing.T) {
	for _, model := range BuiltInModels(true) {
		if model.Provider != ProviderAnthropic {
			continue
		}
		if !strings.Contains(model.ID, "sonnet") && !strings.Contains(model.ID, "opus") {
			continue
		}
		if model.ContextWindow != 1000000 || model.MaxContextWindow != 1000000 {
			t.Fatalf("%s context = %d/%d", model.ID, model.ContextWindow, model.MaxContextWindow)
		}
	}
}

func TestConfigWithAnthropicProviderDefaultsToSonnet(t *testing.T) {
	cfg := (Config{Provider: ProviderAnthropic}).withDefaults()
	if cfg.Model != "claude-sonnet-4-6" {
		t.Fatalf("model = %q", cfg.Model)
	}
	if cfg.Provider != ProviderAnthropic {
		t.Fatalf("provider = %q", cfg.Provider)
	}
	if cfg.Reasoning != ReasoningMedium {
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

func TestEmbeddingModelIsHiddenFromChatModelPickers(t *testing.T) {
	model, ok := LookupModel("text-embedding-3-large")
	if !ok {
		t.Fatal("embedding model missing from catalog")
	}
	if model.Provider != ProviderOpenAI {
		t.Fatalf("provider = %q", model.Provider)
	}
	if model.DefaultReasoning != ReasoningNone || len(model.SupportedReasoning) != 0 {
		t.Fatalf("reasoning = %q %#v", model.DefaultReasoning, model.SupportedReasoning)
	}
	if !model.Hidden {
		t.Fatal("embedding model should be hidden")
	}
	for _, visible := range BuiltInModels(false) {
		if visible.ID == "text-embedding-3-large" {
			t.Fatal("embedding model leaked into visible model picker")
		}
	}
}
