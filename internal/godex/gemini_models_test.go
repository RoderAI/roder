package godex

import (
	"reflect"
	"testing"

	"github.com/pandelisz/gode/internal/godex/provider"
)

func TestGeminiProviderAndModels(t *testing.T) {
	provider, ok := LookupProvider(ProviderGemini)
	if !ok {
		t.Fatal("gemini provider missing")
	}
	if provider.Kind != ProviderKindGemini || provider.EnvKey != "GEMINI_API_TOKEN" || provider.DefaultModel != "gemini-3.1-pro-preview" {
		t.Fatalf("gemini provider = %#v", provider)
	}
	if !reflect.DeepEqual(provider.EnvAliases, []string{"GEMINI_API_KEY", "GOOGLE_API_KEY", "GOOGLE_GENAI_API_KEY", "GOOGLE_AI_API_KEY"}) {
		t.Fatalf("aliases = %#v", provider.EnvAliases)
	}
	for _, id := range []string{"gemini-3.1-pro-preview", "gemini-3.1-pro-preview-customtools", "gemini-3-flash-preview", "gemini-3.1-flash-lite-preview"} {
		model := ModelConfigFor(id)
		if model.Provider != ProviderGemini || model.ContextWindow != 1048576 || model.MaxContextWindow != 1048576 {
			t.Fatalf("%s = %#v", id, model)
		}
		if !model.SupportsImages || !model.SupportsTools || !model.SupportsStructured {
			t.Fatalf("%s capability flags = %#v", id, model)
		}
		if !model.SupportsReasoning(ReasoningMinimal) || !model.SupportsReasoning(ReasoningXHigh) {
			t.Fatalf("%s reasoning = %#v", id, model.SupportedReasoning)
		}
	}
}

func TestConfigWithGeminiProviderDefaultsToPro(t *testing.T) {
	cfg := (Config{Provider: ProviderGemini}).withDefaults()
	if cfg.Model != "gemini-3.1-pro-preview" || cfg.Provider != ProviderGemini || cfg.Reasoning != ReasoningHigh {
		t.Fatalf("cfg = %#v", cfg)
	}
}

func TestResolveSelectedGeminiUsesPreferredEnvAlias(t *testing.T) {
	cfg := (Config{Provider: ProviderGemini}).withDefaults()
	resolved, err := ResolveSelectedModelWithEnv(cfg, map[string]string{
		"GEMINI_API_TOKEN": "preferred",
		"GEMINI_API_KEY":   "secondary",
	})
	if err != nil {
		t.Fatalf("resolve: %v", err)
	}
	if resolved.APIType != provider.APITypeGemini || resolved.APIKeyEnv != "GEMINI_API_TOKEN" || !resolved.HasAPIKey {
		t.Fatalf("resolved = %#v", resolved)
	}
}

func TestGeminiModelsListWithoutCredentials(t *testing.T) {
	cfg := (Config{Provider: ProviderGemini}).withDefaults()
	found := false
	for _, model := range ModelsForConfig(cfg, false) {
		if model.ID == "gemini-3.1-pro-preview" {
			found = true
			break
		}
	}
	if !found {
		t.Fatalf("gemini model missing from listing: %#v", ModelsForConfig(cfg, false))
	}
	resolved, err := ResolveSelectedModelWithEnv(cfg, nil)
	if err != nil {
		t.Fatalf("resolve without key: %v", err)
	}
	if resolved.HasAPIKey || resolved.APIKeyEnv != "GEMINI_API_TOKEN" {
		t.Fatalf("auth should not be required for listing/resolve: %#v", resolved)
	}
}

func TestCustomGeminiModelKeepsLocalIDDistinctFromUpstreamID(t *testing.T) {
	cfg := Config{
		Model: "gemini-pro",
		UserModels: map[string]provider.UserModelConfig{
			"gemini-pro": {
				Type:        string(provider.APITypeGemini),
				Provider:    ProviderGemini,
				Model:       "gemini-3.1-pro-preview",
				DisplayName: "Gemini Pro Alias",
			},
		},
	}
	model, ok := LookupModelForConfig(cfg, "gemini-pro")
	if !ok {
		t.Fatal("custom gemini model missing")
	}
	if model.ID != "gemini-pro" || model.DisplayName != "Gemini Pro Alias" {
		t.Fatalf("model = %#v", model)
	}
	resolved, err := ResolveSelectedModelWithEnv(cfg, map[string]string{"GEMINI_API_TOKEN": "token"})
	if err != nil {
		t.Fatalf("resolve: %v", err)
	}
	if resolved.ID != "gemini-pro" || resolved.UpstreamModel != "gemini-3.1-pro-preview" {
		t.Fatalf("resolved = %#v", resolved)
	}
}

func TestBuildProviderConstructsGeminiBuiltInProvider(t *testing.T) {
	t.Setenv("GEMINI_API_TOKEN", "gemini-secret")
	cfg := (Config{Provider: ProviderGemini}).withDefaults()
	prov, err := buildProvider(cfg)
	if err != nil {
		t.Fatalf("build provider: %v", err)
	}
	if prov.Name() != "gemini" {
		t.Fatalf("provider name = %q", prov.Name())
	}
	value := reflect.ValueOf(prov).Elem()
	if got := value.FieldByName("model").String(); got != "gemini-3.1-pro-preview" {
		t.Fatalf("gemini model = %q", got)
	}
	if got := value.FieldByName("apiKey").String(); got != "gemini-secret" {
		t.Fatalf("gemini api key = %q", got)
	}
}

func TestBuildProviderConstructsGeminiCustomEnterpriseProvider(t *testing.T) {
	cfg := (Config{
		Model: "gemini-enterprise",
		UserModels: map[string]provider.UserModelConfig{
			"gemini-enterprise": {
				Type:     string(provider.APITypeGemini),
				Provider: "gemini-enterprise",
				Model:    "gemini-3.1-pro-preview",
				Backend:  "enterprise",
				Project:  "project-id",
				Location: "us-central1",
			},
		},
	}).withDefaults()
	prov, err := buildProvider(cfg)
	if err != nil {
		t.Fatalf("build provider: %v", err)
	}
	if prov.Name() != "gemini" {
		t.Fatalf("provider name = %q", prov.Name())
	}
	value := reflect.ValueOf(prov).Elem()
	if got := value.FieldByName("backend").String(); got != provider.GeminiBackendEnterprise {
		t.Fatalf("backend = %q", got)
	}
	if got := value.FieldByName("project").String(); got != "project-id" {
		t.Fatalf("project = %q", got)
	}
	if got := value.FieldByName("location").String(); got != "us-central1" {
		t.Fatalf("location = %q", got)
	}
}
