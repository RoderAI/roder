package provider

import "sort"

type ProviderConfig struct {
	ID                 string `json:"id,omitempty" toml:"id,omitempty"`
	Name               string `json:"name,omitempty" toml:"name,omitempty"`
	Kind               string `json:"kind,omitempty" toml:"kind,omitempty"`
	DefaultModel       string `json:"default_model,omitempty" toml:"default_model,omitempty"`
	BaseURL            string `json:"base_url,omitempty" toml:"base_url,omitempty"`
	EnvKey             string `json:"env_key,omitempty" toml:"env_key,omitempty"`
	Disabled           bool   `json:"disabled,omitempty" toml:"disabled,omitempty"`
	RequiresAuth       bool   `json:"requires_auth,omitempty" toml:"requires_auth,omitempty"`
	SupportsWebSockets bool   `json:"supports_websockets,omitempty" toml:"supports_websockets,omitempty"`
}

type SelectedModel struct {
	ID        string `json:"id,omitempty" toml:"id,omitempty"`
	Provider  string `json:"provider,omitempty" toml:"provider,omitempty"`
	Reasoning string `json:"reasoning,omitempty" toml:"reasoning,omitempty"`
}

type ModelMetadata struct {
	ID               string   `json:"id,omitempty" toml:"id,omitempty"`
	DisplayName      string   `json:"display_name,omitempty" toml:"display_name,omitempty"`
	Provider         string   `json:"provider,omitempty" toml:"provider,omitempty"`
	ContextWindow    int      `json:"context_window,omitempty" toml:"context_window,omitempty"`
	SupportsImages   bool     `json:"supports_images,omitempty" toml:"supports_images,omitempty"`
	ReasoningEfforts []string `json:"reasoning_efforts,omitempty" toml:"reasoning_efforts,omitempty"`
	DefaultReasoning string   `json:"default_reasoning,omitempty" toml:"default_reasoning,omitempty"`
	Disabled         bool     `json:"disabled,omitempty" toml:"disabled,omitempty"`
}

type ModelCatalog struct {
	providers []ProviderConfig
	models    []ModelMetadata
}

var Catalog = NewCatalog(defaultProviders(), defaultModels())

func NewCatalog(providers []ProviderConfig, models []ModelMetadata) ModelCatalog {
	return ModelCatalog{
		providers: cloneProviders(providers),
		models:    cloneModels(models),
	}
}

func (c ModelCatalog) Providers(includeDisabled bool) []ProviderConfig {
	out := make([]ProviderConfig, 0, len(c.providers))
	for _, provider := range c.providers {
		if provider.Disabled && !includeDisabled {
			continue
		}
		out = append(out, provider)
	}
	sort.SliceStable(out, func(i, j int) bool { return out[i].ID < out[j].ID })
	return out
}

func (c ModelCatalog) Models(includeDisabled bool) []ModelMetadata {
	out := make([]ModelMetadata, 0, len(c.models))
	for _, model := range c.models {
		if model.Disabled && !includeDisabled {
			continue
		}
		if providerConfig, ok := c.Provider(model.Provider); ok && providerConfig.Disabled && !includeDisabled {
			continue
		}
		out = append(out, cloneModel(model))
	}
	sort.SliceStable(out, func(i, j int) bool {
		if out[i].Provider == out[j].Provider {
			return out[i].ID < out[j].ID
		}
		return out[i].Provider < out[j].Provider
	})
	return out
}

func (c ModelCatalog) Provider(id string) (ProviderConfig, bool) {
	for _, provider := range c.providers {
		if provider.ID == id {
			return provider, true
		}
	}
	return ProviderConfig{}, false
}

func (c ModelCatalog) Model(id string) (ModelMetadata, bool) {
	for _, model := range c.models {
		if model.ID == id {
			return cloneModel(model), true
		}
	}
	return ModelMetadata{}, false
}

func (c ModelCatalog) DefaultModel(providerID string) string {
	providerConfig, ok := c.Provider(providerID)
	if !ok || providerConfig.Disabled {
		return ""
	}
	if providerConfig.DefaultModel != "" {
		if model, ok := c.Model(providerConfig.DefaultModel); ok && !model.Disabled {
			return model.ID
		}
	}
	for _, model := range c.Models(false) {
		if model.Provider == providerID {
			return model.ID
		}
	}
	return ""
}

func (c ModelCatalog) SelectedModel(providerID string) (SelectedModel, bool) {
	modelID := c.DefaultModel(providerID)
	if modelID == "" {
		return SelectedModel{}, false
	}
	model, ok := c.Model(modelID)
	if !ok {
		return SelectedModel{}, false
	}
	return SelectedModel{ID: model.ID, Provider: providerID, Reasoning: model.DefaultReasoning}, true
}

func defaultProviders() []ProviderConfig {
	return []ProviderConfig{
		{ID: "anthropic", Name: "Anthropic", Kind: "anthropic", DefaultModel: "claude-sonnet-4-6", BaseURL: "https://api.anthropic.com", EnvKey: "ANTHROPIC_API_KEY", RequiresAuth: true},
		{ID: "anthropic-compatible", Name: "Anthropic Compatible", Kind: "anthropic-compatible", DefaultModel: "claude-sonnet-4.5", RequiresAuth: true},
		{ID: "openai", Name: "OpenAI", Kind: "openai", DefaultModel: "gpt-5.4-mini", BaseURL: "https://api.openai.com/v1", EnvKey: "OPENAI_API_KEY", RequiresAuth: true, SupportsWebSockets: true},
		{ID: "openai-compatible", Name: "OpenAI Compatible", Kind: "openai-compatible", DefaultModel: "gpt-5.4-mini", RequiresAuth: true},
	}
}

func defaultModels() []ModelMetadata {
	return []ModelMetadata{
		{ID: "claude-opus-4-7", DisplayName: "Claude Opus 4.7", Provider: "anthropic", ContextWindow: 1000000, SupportsImages: true, ReasoningEfforts: standardReasoningEfforts(), DefaultReasoning: "high"},
		{ID: "claude-sonnet-4-6", DisplayName: "Claude Sonnet 4.6", Provider: "anthropic", ContextWindow: 1000000, SupportsImages: true, ReasoningEfforts: standardReasoningEfforts(), DefaultReasoning: "medium"},
		{ID: "claude-haiku-4-5-20251001", DisplayName: "Claude Haiku 4.5", Provider: "anthropic", ContextWindow: 200000, SupportsImages: true, ReasoningEfforts: []string{"low", "medium"}, DefaultReasoning: "low"},
		{ID: "claude-sonnet-4.5", DisplayName: "Claude Sonnet 4.5", Provider: "anthropic-compatible", ContextWindow: 200000, SupportsImages: true, ReasoningEfforts: []string{"none"}, DefaultReasoning: "none"},
		{ID: "gpt-5.3-codex", DisplayName: "GPT-5.3-Codex", Provider: "openai", ContextWindow: 272000, SupportsImages: true, ReasoningEfforts: standardReasoningEfforts(), DefaultReasoning: "medium"},
		{ID: "gpt-5.4", DisplayName: "GPT-5.4", Provider: "openai", ContextWindow: 272000, SupportsImages: true, ReasoningEfforts: standardReasoningEfforts(), DefaultReasoning: "medium"},
		{ID: "gpt-5.4-mini", DisplayName: "GPT-5.4-Mini", Provider: "openai", ContextWindow: 272000, SupportsImages: true, ReasoningEfforts: standardReasoningEfforts(), DefaultReasoning: "medium"},
		{ID: "gpt-5.5", DisplayName: "GPT-5.5", Provider: "openai", ContextWindow: 1050000, SupportsImages: true, ReasoningEfforts: standardReasoningEfforts(), DefaultReasoning: "medium"},
	}
}

func standardReasoningEfforts() []string {
	return []string{"low", "medium", "high", "xhigh"}
}

func cloneProviders(providers []ProviderConfig) []ProviderConfig {
	return append([]ProviderConfig(nil), providers...)
}

func cloneModels(models []ModelMetadata) []ModelMetadata {
	out := make([]ModelMetadata, 0, len(models))
	for _, model := range models {
		out = append(out, cloneModel(model))
	}
	return out
}

func cloneModel(model ModelMetadata) ModelMetadata {
	model.ReasoningEfforts = append([]string(nil), model.ReasoningEfforts...)
	return model
}
