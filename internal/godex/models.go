package godex

const (
	ProviderMock   = "mock"
	ProviderOpenAI = "openai"
	ProviderCodex  = "codex"

	ProviderKindMock   = "mock"
	ProviderKindOpenAI = "openai"

	ReasoningNone    = "none"
	ReasoningMinimal = "minimal"
	ReasoningLow     = "low"
	ReasoningMedium  = "medium"
	ReasoningHigh    = "high"
	ReasoningXHigh   = "xhigh"

	DefaultModelID = "gpt-5.5"
)

type ProviderConfig struct {
	ID                 string
	Name               string
	Kind               string
	DefaultModel       string
	BaseURL            string
	EnvKey             string
	RequiresAuth       bool
	SupportsWebSockets bool
}

type ReasoningOption struct {
	Effort      string
	Description string
}

type ModelConfig struct {
	ID                 string
	DisplayName        string
	Description        string
	Provider           string
	DefaultReasoning   string
	SupportedReasoning []ReasoningOption
	ContextWindow      int
	MaxContextWindow   int
	Hidden             bool
}

func (m ModelConfig) ReasoningEfforts() []string {
	out := make([]string, 0, len(m.SupportedReasoning))
	for _, option := range m.SupportedReasoning {
		out = append(out, option.Effort)
	}
	return out
}

func (m ModelConfig) SupportsReasoning(effort string) bool {
	for _, option := range m.SupportedReasoning {
		if option.Effort == effort {
			return true
		}
	}
	return false
}

func DefaultModelConfig() ModelConfig {
	model, ok := LookupModel(DefaultModelID)
	if ok {
		return model
	}
	return fallbackModelConfig(DefaultModelID)
}

func BuiltInProviders() []ProviderConfig {
	return append([]ProviderConfig(nil), builtInProviders...)
}

func LookupProvider(id string) (ProviderConfig, bool) {
	for _, provider := range builtInProviders {
		if provider.ID == id {
			return provider, true
		}
	}
	return ProviderConfig{}, false
}

func BuiltInModels(includeHidden bool) []ModelConfig {
	out := make([]ModelConfig, 0, len(builtInModels))
	for _, model := range builtInModels {
		if model.Hidden && !includeHidden {
			continue
		}
		out = append(out, cloneModelConfig(model))
	}
	return out
}

func BuiltInModelIDs(includeHidden bool) []string {
	models := BuiltInModels(includeHidden)
	out := make([]string, 0, len(models))
	for _, model := range models {
		out = append(out, model.ID)
	}
	return out
}

func LookupModel(id string) (ModelConfig, bool) {
	for _, model := range builtInModels {
		if model.ID == id {
			return cloneModelConfig(model), true
		}
	}
	return ModelConfig{}, false
}

func ModelConfigFor(id string) ModelConfig {
	if model, ok := LookupModel(id); ok {
		return model
	}
	return fallbackModelConfig(id)
}

func cloneModelConfig(model ModelConfig) ModelConfig {
	model.SupportedReasoning = append([]ReasoningOption(nil), model.SupportedReasoning...)
	return model
}

func fallbackModelConfig(id string) ModelConfig {
	return ModelConfig{
		ID:                 id,
		DisplayName:        id,
		Provider:           ProviderOpenAI,
		DefaultReasoning:   ReasoningMedium,
		SupportedReasoning: standardReasoning(),
		ContextWindow:      272000,
		MaxContextWindow:   272000,
		Hidden:             true,
	}
}

func standardReasoning() []ReasoningOption {
	return []ReasoningOption{
		{Effort: ReasoningLow, Description: "Fast responses with lighter reasoning"},
		{Effort: ReasoningMedium, Description: "Balances speed and reasoning depth for everyday tasks"},
		{Effort: ReasoningHigh, Description: "Greater reasoning depth for complex problems"},
		{Effort: ReasoningXHigh, Description: "Extra high reasoning depth for complex problems"},
	}
}

var builtInProviders = []ProviderConfig{
	{
		ID:                 ProviderMock,
		Name:               "Mock",
		Kind:               ProviderKindMock,
		DefaultModel:       "mock",
		RequiresAuth:       false,
		SupportsWebSockets: false,
	},
	{
		ID:                 ProviderOpenAI,
		Name:               "OpenAI",
		Kind:               ProviderKindOpenAI,
		DefaultModel:       DefaultModelID,
		BaseURL:            "https://api.openai.com/v1",
		EnvKey:             "OPENAI_API_KEY",
		RequiresAuth:       true,
		SupportsWebSockets: true,
	},
	{
		ID:                 ProviderCodex,
		Name:               "Codex",
		Kind:               ProviderKindOpenAI,
		DefaultModel:       DefaultModelID,
		BaseURL:            "https://api.openai.com/v1",
		EnvKey:             "OPENAI_API_KEY",
		RequiresAuth:       true,
		SupportsWebSockets: true,
	},
}

var builtInModels = []ModelConfig{
	{
		ID:                 "gpt-5.5",
		DisplayName:        "GPT-5.5",
		Description:        "Frontier model for complex coding, research, and real-world work.",
		Provider:           ProviderOpenAI,
		DefaultReasoning:   ReasoningMedium,
		SupportedReasoning: standardReasoning(),
		ContextWindow:      272000,
		MaxContextWindow:   272000,
	},
	{
		ID:                 "gpt-5.4-mini",
		DisplayName:        "GPT-5.4-Mini",
		Description:        "Small, fast, and cost-efficient model for simpler coding tasks.",
		Provider:           ProviderOpenAI,
		DefaultReasoning:   ReasoningMedium,
		SupportedReasoning: standardReasoning(),
		ContextWindow:      272000,
		MaxContextWindow:   272000,
	},
	{
		ID:                 "gpt-5.4",
		DisplayName:        "GPT-5.4",
		Description:        "Strong model for everyday coding.",
		Provider:           ProviderOpenAI,
		DefaultReasoning:   ReasoningMedium,
		SupportedReasoning: standardReasoning(),
		ContextWindow:      272000,
		MaxContextWindow:   272000,
	},
	{
		ID:                 "gpt-5.3-codex",
		DisplayName:        "GPT-5.3-Codex",
		Description:        "Coding-optimized model.",
		Provider:           ProviderOpenAI,
		DefaultReasoning:   ReasoningMedium,
		SupportedReasoning: standardReasoning(),
		ContextWindow:      272000,
		MaxContextWindow:   272000,
	},
	{
		ID:               "gpt-5.2",
		DisplayName:      "GPT-5.2",
		Description:      "Optimized for professional work and long-running agents.",
		Provider:         ProviderOpenAI,
		DefaultReasoning: ReasoningMedium,
		SupportedReasoning: []ReasoningOption{
			{Effort: ReasoningLow, Description: "Balances speed with some reasoning; useful for straightforward queries and short explanations"},
			{Effort: ReasoningMedium, Description: "Provides a solid balance of reasoning depth and latency for general-purpose tasks"},
			{Effort: ReasoningHigh, Description: "Maximizes reasoning depth for complex or ambiguous problems"},
			{Effort: ReasoningXHigh, Description: "Extra high reasoning for complex problems"},
		},
		ContextWindow:    272000,
		MaxContextWindow: 272000,
	},
	{
		ID:                 "codex-auto-review",
		DisplayName:        "Codex Auto Review",
		Description:        "Automatic approval review model for Codex.",
		Provider:           ProviderOpenAI,
		DefaultReasoning:   ReasoningMedium,
		SupportedReasoning: standardReasoning(),
		ContextWindow:      272000,
		MaxContextWindow:   272000,
		Hidden:             true,
	},
	{
		ID:                 "mock",
		DisplayName:        "Mock",
		Description:        "Local deterministic mock provider for tests and offline development.",
		Provider:           ProviderMock,
		DefaultReasoning:   ReasoningNone,
		SupportedReasoning: []ReasoningOption{{Effort: ReasoningNone, Description: "No model-side reasoning"}},
		Hidden:             true,
	},
}
