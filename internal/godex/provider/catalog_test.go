package provider

import (
	"reflect"
	"strings"
	"testing"
)

func TestCatalogDefaultModel(t *testing.T) {
	if got := Catalog.DefaultModel("openai"); got != "gpt-5.4-mini" {
		t.Fatalf("openai default model = %q", got)
	}
	if got := Catalog.DefaultModel("missing"); got != "" {
		t.Fatalf("missing provider default model = %q", got)
	}
}

func TestCatalogLookup(t *testing.T) {
	providerConfig, ok := Catalog.Provider("openai")
	if !ok {
		t.Fatal("openai provider missing")
	}
	if providerConfig.Kind != "openai" || providerConfig.BaseURL == "" {
		t.Fatalf("openai provider = %#v", providerConfig)
	}

	model, ok := Catalog.Model("gpt-5.5")
	if !ok {
		t.Fatal("gpt-5.5 model missing")
	}
	if model.Provider != "openai" || model.ContextWindow == 0 || !model.SupportsImages || model.EditTool != "patch" {
		t.Fatalf("gpt-5.5 model = %#v", model)
	}
}

func TestCatalogFiltersDisabledProviders(t *testing.T) {
	catalog := NewCatalog([]ProviderConfig{
		{ID: "enabled", DefaultModel: "enabled-model"},
		{ID: "disabled", DefaultModel: "disabled-model", Disabled: true},
	}, []ModelMetadata{
		{ID: "enabled-model", Provider: "enabled"},
		{ID: "disabled-model", Provider: "disabled"},
	})

	if providers := catalog.Providers(false); len(providers) != 1 || providers[0].ID != "enabled" {
		t.Fatalf("enabled providers = %#v", providers)
	}
	if models := catalog.Models(false); len(models) != 1 || models[0].ID != "enabled-model" {
		t.Fatalf("enabled models = %#v", models)
	}
	if got := catalog.DefaultModel("disabled"); got != "" {
		t.Fatalf("disabled provider default = %q", got)
	}
}

func TestSelectedModelUsesDefaultReasoning(t *testing.T) {
	selected, ok := Catalog.SelectedModel("openai")
	if !ok {
		t.Fatal("selected model missing")
	}
	if selected.ID != "gpt-5.4-mini" || selected.Provider != "openai" || selected.Reasoning != "medium" {
		t.Fatalf("selected model = %#v", selected)
	}
}

func TestCatalogListsAreStableSortedCopies(t *testing.T) {
	catalog := NewCatalog([]ProviderConfig{
		{ID: "z"},
		{ID: "a"},
	}, []ModelMetadata{
		{ID: "z-model", Provider: "z", ReasoningEfforts: []string{"medium"}},
		{ID: "a-model", Provider: "a", ReasoningEfforts: []string{"low"}},
	})

	providers := catalog.Providers(true)
	if got := []string{providers[0].ID, providers[1].ID}; !reflect.DeepEqual(got, []string{"a", "z"}) {
		t.Fatalf("providers order = %#v", got)
	}
	models := catalog.Models(true)
	if got := []string{models[0].ID, models[1].ID}; !reflect.DeepEqual(got, []string{"a-model", "z-model"}) {
		t.Fatalf("models order = %#v", got)
	}

	models[0].ReasoningEfforts[0] = "mutated"
	again, _ := catalog.Model("a-model")
	if again.ReasoningEfforts[0] != "low" {
		t.Fatalf("catalog returned aliased model: %#v", again)
	}
}

func TestDefaultCatalogSonnetAndOpusModelsUseMillionTokenContext(t *testing.T) {
	for _, model := range Catalog.Models(false) {
		if !strings.Contains(model.ID, "claude-sonnet") && !strings.Contains(model.ID, "claude-opus") {
			continue
		}
		if model.ContextWindow != 1000000 {
			t.Fatalf("%s context = %d", model.ID, model.ContextWindow)
		}
	}
}

func TestCatalogWithUserModelsAddsCustomEntries(t *testing.T) {
	catalog, err := Catalog.WithConfig(nil, map[string]UserModelConfig{
		"deepseek-chat": {
			Type:          string(APITypeChatCompletions),
			Provider:      "deepseek",
			Model:         "deepseek-chat",
			DisplayName:   "DeepSeek Chat",
			BaseURL:       "https://api.deepseek.example/v1",
			APIKeyEnv:     "DEEPSEEK_API_KEY",
			ContextWindow: 128000,
			EditTool:      "edit",
		},
		"kimi-k2-6": {
			Type:          string(APITypeChatCompletions),
			Provider:      "moonshot",
			Model:         "kimi-k2.6",
			DisplayName:   "Kimi K2.6",
			BaseURL:       "https://api.moonshot.example/v1",
			ContextWindow: 262144,
		},
	}, map[string]string{"DEEPSEEK_API_KEY": "secret"})
	if err != nil {
		t.Fatalf("merge catalog: %v", err)
	}
	deepseek, ok := catalog.Model("deepseek-chat")
	if !ok {
		t.Fatal("deepseek-chat missing")
	}
	if deepseek.Provider != "deepseek" || deepseek.DisplayName != "DeepSeek Chat" || deepseek.ContextWindow != 128000 || deepseek.EditTool != "edit" {
		t.Fatalf("deepseek metadata = %#v", deepseek)
	}
	kimi, ok := catalog.Model("kimi-k2-6")
	if !ok {
		t.Fatal("kimi-k2-6 missing")
	}
	if kimi.Provider != "moonshot" || kimi.DisplayName != "Kimi K2.6" || kimi.ContextWindow != 262144 {
		t.Fatalf("kimi metadata = %#v", kimi)
	}
	if got := catalog.DefaultModel("deepseek"); got != "deepseek-chat" {
		t.Fatalf("deepseek default = %q", got)
	}
}

func TestCatalogWithUserModelsHidesDisabledEntries(t *testing.T) {
	catalog, err := Catalog.WithConfig(nil, map[string]UserModelConfig{
		"hidden-local": {
			Type:     string(APITypeChatCompletions),
			Provider: "local",
			Disabled: true,
		},
	}, nil)
	if err != nil {
		t.Fatalf("merge catalog: %v", err)
	}
	for _, model := range catalog.Models(false) {
		if model.ID == "hidden-local" {
			t.Fatal("disabled custom model visible")
		}
	}
	model, ok := catalog.Model("hidden-local")
	if !ok {
		t.Fatal("disabled custom model should remain addressable in include-disabled catalog")
	}
	if !model.Disabled {
		t.Fatalf("model disabled = false: %#v", model)
	}
}

func TestCatalogWithUserModelsOverridesBuiltInOnlyWithProvider(t *testing.T) {
	catalog, err := Catalog.WithConfig(nil, map[string]UserModelConfig{
		"gpt-5.5": {
			Type:             string(APITypeResponses),
			Provider:         "openai-compatible",
			Model:            "router-gpt-5.5",
			DisplayName:      "Router GPT-5.5",
			ContextWindow:    200000,
			DefaultReasoning: "high",
			ReasoningEfforts: []string{"medium", "high"},
		},
	}, nil)
	if err != nil {
		t.Fatalf("merge catalog: %v", err)
	}
	model, ok := catalog.Model("gpt-5.5")
	if !ok {
		t.Fatal("gpt-5.5 missing")
	}
	if model.Provider != "openai-compatible" || model.DisplayName != "Router GPT-5.5" || model.ContextWindow != 200000 || model.DefaultReasoning != "high" {
		t.Fatalf("override model = %#v", model)
	}
	if !reflect.DeepEqual(model.ReasoningEfforts, []string{"medium", "high"}) {
		t.Fatalf("reasoning efforts = %#v", model.ReasoningEfforts)
	}

	catalog, err = Catalog.WithConfig(nil, map[string]UserModelConfig{
		"gpt-5.5": {DisplayName: "Ignored"},
	}, nil)
	if err != nil {
		t.Fatalf("merge catalog without provider: %v", err)
	}
	model, _ = catalog.Model("gpt-5.5")
	if model.DisplayName == "Ignored" {
		t.Fatal("built-in model override without provider should be ignored")
	}
}

func TestCatalogWithConfigKeepsBuiltInDefaultsWithoutUserModels(t *testing.T) {
	catalog, err := Catalog.WithConfig(nil, nil, nil)
	if err != nil {
		t.Fatalf("merge catalog: %v", err)
	}
	got, _ := catalog.Model("gpt-5.5")
	want, _ := Catalog.Model("gpt-5.5")
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("built-in changed:\ngot  %#v\nwant %#v", got, want)
	}
}
