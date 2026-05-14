package godex

import (
	"fmt"
	"os"
	"sort"
	"strings"

	"github.com/pandelisz/gode/internal/godex/contextwindow"
	"github.com/pandelisz/gode/internal/godex/provider"
)

func LookupProviderForConfig(cfg Config, id string) (ProviderConfig, bool) {
	providers := providerConfigsForConfig(cfg)
	providerConfig, ok := providers[id]
	return providerConfig, ok
}

func ModelsForConfig(cfg Config, includeHidden bool) []ModelConfig {
	models := modelConfigsForConfig(cfg)
	providers := providerConfigsForConfig(cfg)
	out := make([]ModelConfig, 0, len(models))
	seen := map[string]bool{}
	for _, builtIn := range builtInModels {
		model, ok := models[builtIn.ID]
		if !ok {
			continue
		}
		if includeModelForConfig(model, providers, includeHidden) {
			out = append(out, cloneModelConfig(model))
		}
		seen[model.ID] = true
	}
	customIDs := make([]string, 0, len(models))
	for id := range models {
		if !seen[id] {
			customIDs = append(customIDs, id)
		}
	}
	sort.Strings(customIDs)
	for _, id := range customIDs {
		model := models[id]
		if includeModelForConfig(model, providers, includeHidden) {
			out = append(out, cloneModelConfig(model))
		}
	}
	return out
}

func includeModelForConfig(model ModelConfig, providers map[string]ProviderConfig, includeHidden bool) bool {
	if model.Hidden && !includeHidden {
		return false
	}
	if providerConfig, ok := providers[model.Provider]; ok && providerConfig.Disabled && !includeHidden {
		return false
	}
	return true
}

func sortedModelsForConfig(cfg Config, includeHidden bool) []ModelConfig {
	models := modelConfigsForConfig(cfg)
	providers := providerConfigsForConfig(cfg)
	out := make([]ModelConfig, 0, len(models))
	for _, model := range models {
		if model.Hidden && !includeHidden {
			continue
		}
		if providerConfig, ok := providers[model.Provider]; ok && providerConfig.Disabled && !includeHidden {
			continue
		}
		out = append(out, cloneModelConfig(model))
	}
	sort.SliceStable(out, func(i, j int) bool {
		if out[i].Provider == out[j].Provider {
			return out[i].ID < out[j].ID
		}
		return out[i].Provider < out[j].Provider
	})
	return out
}

func LookupModelForConfig(cfg Config, id string) (ModelConfig, bool) {
	model, ok := modelConfigsForConfig(cfg)[id]
	if !ok {
		return ModelConfig{}, false
	}
	return cloneModelConfig(model), true
}

func ModelConfigForConfig(cfg Config, id string) ModelConfig {
	if model, ok := LookupModelForConfig(cfg, id); ok {
		return model
	}
	return fallbackModelConfig(id)
}

func ResolveSelectedModel(cfg Config) (provider.ResolvedModel, error) {
	return ResolveSelectedModelWithEnv(cfg, envMapFromOS())
}

func ResolveSelectedModelWithEnv(cfg Config, env map[string]string) (provider.ResolvedModel, error) {
	if userModel, ok := cfg.UserModels[cfg.Model]; ok {
		resolved, err := provider.ResolveUserModel(cfg.Model, userModel, env)
		if err != nil {
			return provider.ResolvedModel{}, fmt.Errorf("resolve model %q: %w", cfg.Model, err)
		}
		return resolved, nil
	}
	model := ModelConfigForConfig(cfg, cfg.Model)
	providerConfig, ok := LookupProviderForConfig(cfg, model.Provider)
	if !ok {
		return provider.ResolvedModel{}, fmt.Errorf("unknown provider %q for model %q", model.Provider, cfg.Model)
	}
	return provider.ResolvedModel{
		ID:            model.ID,
		UpstreamModel: model.ID,
		APIType:       apiTypeForProviderKind(providerConfig.Kind),
		ProviderID:    providerConfig.ID,
		BaseURL:       providerConfig.BaseURL,
		APIKeyEnv:     providerConfig.EnvKey,
		Metadata: provider.ModelMetadata{
			ID:               model.ID,
			DisplayName:      model.DisplayName,
			Provider:         model.Provider,
			ContextWindow:    model.ContextWindow,
			ReasoningEfforts: model.ReasoningEfforts(),
			DefaultReasoning: model.DefaultReasoning,
			Disabled:         model.Hidden,
		},
		EditTool: model.EditTool,
	}, nil
}

func envMapFromOS() map[string]string {
	out := map[string]string{}
	for _, pair := range os.Environ() {
		key, value, ok := strings.Cut(pair, "=")
		if ok {
			out[key] = value
		}
	}
	return out
}

func modelConfigsForConfig(cfg Config) map[string]ModelConfig {
	out := make(map[string]ModelConfig, len(builtInModels)+len(cfg.UserModels))
	for _, model := range builtInModels {
		out[model.ID] = cloneModelConfig(model)
	}
	for id, userModel := range cfg.UserModels {
		if _, builtIn := out[id]; builtIn && strings.TrimSpace(userModel.Provider) == "" {
			continue
		}
		model, err := modelConfigFromUserModel(id, userModel)
		if err != nil {
			continue
		}
		out[id] = model
	}
	return out
}

func providerConfigsForConfig(cfg Config) map[string]ProviderConfig {
	out := make(map[string]ProviderConfig, len(builtInProviders)+len(cfg.ProviderConfig)+len(cfg.UserModels))
	for _, providerConfig := range builtInProviders {
		out[providerConfig.ID] = providerConfig
	}
	for id, providerConfig := range cfg.ProviderConfig {
		out[id] = providerConfigFromProviderPackage(id, providerConfig)
	}
	for id, userModel := range cfg.UserModels {
		resolved, err := provider.ResolveUserModel(id, userModel, nil)
		if err != nil {
			continue
		}
		if existing, ok := out[resolved.ProviderID]; ok {
			if existing.DefaultModel == "" {
				existing.DefaultModel = id
			}
			out[resolved.ProviderID] = existing
			continue
		}
		out[resolved.ProviderID] = ProviderConfig{
			ID:           resolved.ProviderID,
			Name:         displayName(resolved.ProviderID),
			Kind:         providerKindForAPIType(resolved.APIType),
			DefaultModel: id,
			BaseURL:      resolved.BaseURL,
			EnvKey:       resolved.APIKeyEnv,
			RequiresAuth: resolved.APIKey != "" || resolved.APIKeyEnv != "",
		}
	}
	return out
}

func providerConfigFromProviderPackage(id string, cfg provider.ProviderConfig) ProviderConfig {
	providerID := strings.TrimSpace(cfg.ID)
	if providerID == "" {
		providerID = id
	}
	name := strings.TrimSpace(cfg.Name)
	if name == "" {
		name = displayName(providerID)
	}
	return ProviderConfig{
		ID:                 providerID,
		Name:               name,
		Kind:               strings.TrimSpace(cfg.Kind),
		DefaultModel:       strings.TrimSpace(cfg.DefaultModel),
		BaseURL:            strings.TrimSpace(cfg.BaseURL),
		EnvKey:             strings.TrimSpace(cfg.EnvKey),
		RequiresAuth:       cfg.RequiresAuth,
		SupportsWebSockets: cfg.SupportsWebSockets,
		Disabled:           cfg.Disabled,
	}
}

func modelConfigFromUserModel(id string, cfg provider.UserModelConfig) (ModelConfig, error) {
	resolved, err := provider.ResolveUserModel(id, cfg, nil)
	if err != nil {
		return ModelConfig{}, err
	}
	window := contextwindow.ForModel(resolved.UpstreamModel)
	if cfg.ContextWindow > 0 {
		window.ContextWindow = cfg.ContextWindow
		window.AutoCompactTokenLimit = int(float64(cfg.ContextWindow) * 0.8)
	}
	if cfg.MaxContextWindow > 0 {
		window.MaxContextWindow = cfg.MaxContextWindow
	} else if cfg.ContextWindow > 0 {
		window.MaxContextWindow = cfg.ContextWindow
	}
	defaultReasoning := strings.TrimSpace(cfg.DefaultReasoning)
	if defaultReasoning == "" {
		defaultReasoning = ReasoningNone
	}
	return ModelConfig{
		ID:                    resolved.ID,
		DisplayName:           firstNonEmpty(strings.TrimSpace(cfg.DisplayName), resolved.ID),
		Description:           fmt.Sprintf("%s custom model", resolved.APIType),
		Provider:              resolved.ProviderID,
		DefaultReasoning:      defaultReasoning,
		SupportedReasoning:    reasoningOptionsFromEfforts(cfg.ReasoningEfforts, defaultReasoning),
		ContextWindow:         window.ContextWindow,
		MaxContextWindow:      window.MaxContextWindow,
		AutoCompactTokenLimit: window.AutoCompactTokenLimit,
		SupportsCompaction:    cfg.SupportsCompaction,
		EditTool:              normalizeEditTool(firstNonEmpty(resolved.EditTool, defaultEditToolForModel(resolved.UpstreamModel))),
		Hidden:                cfg.Disabled,
	}, nil
}

func reasoningOptionsFromEfforts(efforts []string, defaultReasoning string) []ReasoningOption {
	if len(efforts) == 0 {
		return []ReasoningOption{{Effort: defaultReasoning, Description: "Configured default reasoning"}}
	}
	out := make([]ReasoningOption, 0, len(efforts))
	for _, effort := range efforts {
		effort = strings.TrimSpace(effort)
		if effort == "" {
			continue
		}
		out = append(out, ReasoningOption{Effort: effort, Description: "Configured reasoning effort"})
	}
	if len(out) == 0 {
		return []ReasoningOption{{Effort: defaultReasoning, Description: "Configured default reasoning"}}
	}
	return out
}

func providerKindForAPIType(apiType provider.APIType) string {
	switch apiType {
	case provider.APITypeAnthropic:
		return ProviderKindAnthropic
	case provider.APITypeChatCompletions:
		return ProviderKindChat
	case provider.APITypeResponses:
		return ProviderKindOpenAI
	default:
		return string(apiType)
	}
}

func apiTypeForProviderKind(kind string) provider.APIType {
	switch kind {
	case ProviderKindAnthropic:
		return provider.APITypeAnthropic
	case ProviderKindChat:
		return provider.APITypeChatCompletions
	case ProviderKindOpenAI:
		return provider.APITypeResponses
	default:
		return provider.APITypeResponses
	}
}

func displayName(id string) string {
	if id == "" {
		return ""
	}
	parts := strings.FieldsFunc(id, func(r rune) bool { return r == '-' || r == '_' })
	for i, part := range parts {
		if part == "" {
			continue
		}
		parts[i] = strings.ToUpper(part[:1]) + part[1:]
	}
	return strings.Join(parts, " ")
}

func firstNonEmpty(values ...string) string {
	for _, value := range values {
		if strings.TrimSpace(value) != "" {
			return strings.TrimSpace(value)
		}
	}
	return ""
}
