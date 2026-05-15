package provider

import (
	"sort"
	"strings"
)

type ProviderConfig struct {
	ID                 string   `json:"id,omitempty" toml:"id,omitempty"`
	Name               string   `json:"name,omitempty" toml:"name,omitempty"`
	Kind               string   `json:"kind,omitempty" toml:"kind,omitempty"`
	DefaultModel       string   `json:"default_model,omitempty" toml:"default_model,omitempty"`
	BaseURL            string   `json:"base_url,omitempty" toml:"base_url,omitempty"`
	EnvKey             string   `json:"env_key,omitempty" toml:"env_key,omitempty"`
	EnvAliases         []string `json:"env_aliases,omitempty" toml:"env_aliases,omitempty"`
	Disabled           bool     `json:"disabled,omitempty" toml:"disabled,omitempty"`
	RequiresAuth       bool     `json:"requires_auth,omitempty" toml:"requires_auth,omitempty"`
	SupportsWebSockets bool     `json:"supports_websockets,omitempty" toml:"supports_websockets,omitempty"`
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
	SupportsTools    bool     `json:"supports_tools,omitempty" toml:"supports_tools,omitempty"`
	SupportsJSON     bool     `json:"supports_json,omitempty" toml:"supports_json,omitempty"`
	ReasoningEfforts []string `json:"reasoning_efforts,omitempty" toml:"reasoning_efforts,omitempty"`
	DefaultReasoning string   `json:"default_reasoning,omitempty" toml:"default_reasoning,omitempty"`
	EditTool         string   `json:"edit_tool,omitempty" toml:"edit_tool,omitempty"`
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

func (c ModelCatalog) WithConfig(providerConfigs map[string]ProviderConfig, userModels map[string]UserModelConfig, env map[string]string) (ModelCatalog, error) {
	providers := map[string]ProviderConfig{}
	for _, cfg := range c.providers {
		providers[cfg.ID] = cfg
	}
	providerIDs := make([]string, 0, len(providerConfigs))
	for id := range providerConfigs {
		providerIDs = append(providerIDs, id)
	}
	sort.Strings(providerIDs)
	for _, id := range providerIDs {
		cfg := providerConfigs[id]
		if strings.TrimSpace(cfg.ID) == "" {
			cfg.ID = id
		}
		providers[cfg.ID] = cfg
	}

	models := map[string]ModelMetadata{}
	for _, model := range c.models {
		models[model.ID] = cloneModel(model)
	}
	modelIDs := make([]string, 0, len(userModels))
	for id := range userModels {
		modelIDs = append(modelIDs, id)
	}
	sort.Strings(modelIDs)
	for _, id := range modelIDs {
		cfg := userModels[id]
		resolved, err := ResolveUserModel(id, cfg, env)
		if err != nil {
			return ModelCatalog{}, err
		}
		base, hadBase := models[id]
		if hadBase && strings.TrimSpace(cfg.Provider) == "" {
			continue
		}
		metadata := mergeModelMetadata(base, resolved.Metadata)
		models[id] = metadata
		if _, ok := providers[resolved.ProviderID]; !ok {
			providers[resolved.ProviderID] = ProviderConfig{
				ID:           resolved.ProviderID,
				Name:         resolved.ProviderID,
				Kind:         string(resolved.APIType),
				DefaultModel: id,
				BaseURL:      resolved.BaseURL,
				EnvKey:       resolved.APIKeyEnv,
				RequiresAuth: resolved.APIKey != "" || resolved.APIKeyEnv != "",
			}
		}
	}

	return NewCatalog(providerMapValues(providers), modelMapValues(models)), nil
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
		{ID: "gemini", Name: "Gemini", Kind: "gemini", DefaultModel: "gemini-3.1-pro-preview", EnvKey: "GEMINI_API_TOKEN", EnvAliases: geminiEnvAliases(), RequiresAuth: true},
		{ID: "openai", Name: "OpenAI", Kind: "openai", DefaultModel: "gpt-5.4-mini", BaseURL: "https://api.openai.com/v1", EnvKey: "OPENAI_API_KEY", RequiresAuth: true, SupportsWebSockets: true},
		{ID: "openai-compatible", Name: "OpenAI Compatible", Kind: "openai-compatible", DefaultModel: "gpt-5.4-mini", RequiresAuth: true},
	}
}

func defaultModels() []ModelMetadata {
	return []ModelMetadata{
		{ID: "claude-opus-4-7", DisplayName: "Claude Opus 4.7", Provider: "anthropic", ContextWindow: 1000000, SupportsImages: true, ReasoningEfforts: standardReasoningEfforts(), DefaultReasoning: "high", EditTool: "edit"},
		{ID: "claude-sonnet-4-6", DisplayName: "Claude Sonnet 4.6", Provider: "anthropic", ContextWindow: 1000000, SupportsImages: true, ReasoningEfforts: standardReasoningEfforts(), DefaultReasoning: "medium", EditTool: "edit"},
		{ID: "claude-haiku-4-5-20251001", DisplayName: "Claude Haiku 4.5", Provider: "anthropic", ContextWindow: 200000, SupportsImages: true, ReasoningEfforts: []string{"low", "medium"}, DefaultReasoning: "low", EditTool: "edit"},
		{ID: "gemini-3-flash-preview", DisplayName: "Gemini 3 Flash Preview", Provider: "gemini", ContextWindow: 1048576, SupportsImages: true, SupportsTools: true, SupportsJSON: true, ReasoningEfforts: geminiReasoningEfforts(), DefaultReasoning: "medium", EditTool: "edit"},
		{ID: "gemini-3.1-flash-lite-preview", DisplayName: "Gemini 3.1 Flash-Lite Preview", Provider: "gemini", ContextWindow: 1048576, SupportsImages: true, SupportsTools: true, SupportsJSON: true, ReasoningEfforts: geminiReasoningEfforts(), DefaultReasoning: "low", EditTool: "edit"},
		{ID: "gemini-3.1-pro-preview", DisplayName: "Gemini 3.1 Pro Preview", Provider: "gemini", ContextWindow: 1048576, SupportsImages: true, SupportsTools: true, SupportsJSON: true, ReasoningEfforts: geminiReasoningEfforts(), DefaultReasoning: "high", EditTool: "edit"},
		{ID: "gemini-3.1-pro-preview-customtools", DisplayName: "Gemini 3.1 Pro Preview Custom Tools", Provider: "gemini", ContextWindow: 1048576, SupportsImages: true, SupportsTools: true, SupportsJSON: true, ReasoningEfforts: geminiReasoningEfforts(), DefaultReasoning: "high", EditTool: "edit"},
		{ID: "gpt-5.3-codex", DisplayName: "GPT-5.3-Codex", Provider: "openai", ContextWindow: 272000, SupportsImages: true, ReasoningEfforts: standardReasoningEfforts(), DefaultReasoning: "medium", EditTool: "patch"},
		{ID: "gpt-5.4", DisplayName: "GPT-5.4", Provider: "openai", ContextWindow: 272000, SupportsImages: true, ReasoningEfforts: standardReasoningEfforts(), DefaultReasoning: "medium", EditTool: "patch"},
		{ID: "gpt-5.4-mini", DisplayName: "GPT-5.4-Mini", Provider: "openai", ContextWindow: 272000, SupportsImages: true, ReasoningEfforts: standardReasoningEfforts(), DefaultReasoning: "medium", EditTool: "patch"},
		{ID: "gpt-5.5", DisplayName: "GPT-5.5", Provider: "openai", ContextWindow: 1050000, SupportsImages: true, ReasoningEfforts: standardReasoningEfforts(), DefaultReasoning: "medium", EditTool: "patch"},
	}
}

func standardReasoningEfforts() []string {
	return []string{"low", "medium", "high", "xhigh"}
}

func geminiReasoningEfforts() []string {
	return []string{"none", "minimal", "low", "medium", "high", "xhigh"}
}

func geminiEnvAliases() []string {
	return append([]string(nil), DefaultGeminiEnvAliases[1:]...)
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

func mergeModelMetadata(base ModelMetadata, override ModelMetadata) ModelMetadata {
	out := cloneModel(base)
	if override.ID != "" {
		out.ID = override.ID
	}
	if override.DisplayName != "" {
		out.DisplayName = override.DisplayName
	}
	if override.Provider != "" {
		out.Provider = override.Provider
	}
	if override.ContextWindow > 0 {
		out.ContextWindow = override.ContextWindow
	}
	if override.SupportsImages {
		out.SupportsImages = true
	}
	if override.SupportsTools {
		out.SupportsTools = true
	}
	if override.SupportsJSON {
		out.SupportsJSON = true
	}
	if len(override.ReasoningEfforts) > 0 {
		out.ReasoningEfforts = append([]string(nil), override.ReasoningEfforts...)
	}
	if override.DefaultReasoning != "" {
		out.DefaultReasoning = override.DefaultReasoning
	}
	if override.EditTool != "" {
		out.EditTool = override.EditTool
	}
	out.Disabled = override.Disabled
	if out.DisplayName == "" {
		out.DisplayName = out.ID
	}
	return out
}

func providerMapValues(in map[string]ProviderConfig) []ProviderConfig {
	out := make([]ProviderConfig, 0, len(in))
	for _, cfg := range in {
		out = append(out, cfg)
	}
	return out
}

func modelMapValues(in map[string]ModelMetadata) []ModelMetadata {
	out := make([]ModelMetadata, 0, len(in))
	for _, model := range in {
		out = append(out, model)
	}
	return out
}
