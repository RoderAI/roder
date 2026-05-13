package provider

import (
	"reflect"
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
	if model.Provider != "openai" || model.ContextWindow == 0 || !model.SupportsImages {
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
