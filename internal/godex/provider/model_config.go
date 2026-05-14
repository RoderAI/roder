package provider

import (
	"fmt"
	"sort"
	"strings"
)

type APIType string

const (
	APITypeResponses       APIType = "responses"
	APITypeChatCompletions APIType = "chat_completions"
	APITypeAnthropic       APIType = "anthropic"
)

type UserModelConfig struct {
	ID                 string            `json:"-" toml:"-"`
	Type               string            `json:"type,omitempty" toml:"type,omitempty"`
	Provider           string            `json:"provider,omitempty" toml:"provider,omitempty"`
	Model              string            `json:"model,omitempty" toml:"model,omitempty"`
	DisplayName        string            `json:"display_name,omitempty" toml:"display_name,omitempty"`
	BaseURL            string            `json:"base_url,omitempty" toml:"base_url,omitempty"`
	APIKey             string            `json:"api_key,omitempty" toml:"api_key,omitempty"`
	APIKeyEnv          string            `json:"api_key_env,omitempty" toml:"api_key_env,omitempty"`
	Headers            map[string]string `json:"headers,omitempty" toml:"headers,omitempty"`
	HeaderEnv          map[string]string `json:"header_env,omitempty" toml:"header_env,omitempty"`
	ContextWindow      int               `json:"context_window,omitempty" toml:"context_window,omitempty"`
	MaxContextWindow   int               `json:"max_context_window,omitempty" toml:"max_context_window,omitempty"`
	DefaultReasoning   string            `json:"default_reasoning,omitempty" toml:"default_reasoning,omitempty"`
	ReasoningEfforts   []string          `json:"reasoning_efforts,omitempty" toml:"reasoning_efforts,omitempty"`
	SupportsImages     bool              `json:"supports_images,omitempty" toml:"supports_images,omitempty"`
	SupportsTools      *bool             `json:"supports_tools,omitempty" toml:"supports_tools,omitempty"`
	SupportsCompaction bool              `json:"supports_compaction,omitempty" toml:"supports_compaction,omitempty"`
	Disabled           bool              `json:"disabled,omitempty" toml:"disabled,omitempty"`
}

type ResolvedModel struct {
	ID            string
	UpstreamModel string
	APIType       APIType
	ProviderID    string
	BaseURL       string
	APIKey        string
	APIKeyEnv     string
	HasAPIKey     bool
	Headers       map[string]string
	HeaderEnv     map[string]string
	Metadata      ModelMetadata
	SupportsTools *bool
}

func ResolveUserModel(id string, cfg UserModelConfig, env map[string]string) (ResolvedModel, error) {
	id = strings.TrimSpace(id)
	if id == "" {
		return ResolvedModel{}, fmt.Errorf("model id is required")
	}
	apiType, err := parseAPIType(cfg.Type)
	if err != nil {
		return ResolvedModel{}, err
	}
	upstreamModel := strings.TrimSpace(cfg.Model)
	if upstreamModel == "" {
		upstreamModel = id
	}
	providerID := strings.TrimSpace(cfg.Provider)
	if providerID == "" {
		providerID = string(apiType)
	}
	apiKey, apiKeyEnv, hasAPIKey := resolveAPIKey(cfg, env)
	return ResolvedModel{
		ID:            id,
		UpstreamModel: upstreamModel,
		APIType:       apiType,
		ProviderID:    providerID,
		BaseURL:       strings.TrimSpace(cfg.BaseURL),
		APIKey:        apiKey,
		APIKeyEnv:     apiKeyEnv,
		HasAPIKey:     hasAPIKey,
		Headers:       cloneStringMap(cfg.Headers),
		HeaderEnv:     cloneStringMap(cfg.HeaderEnv),
		Metadata: ModelMetadata{
			ID:               id,
			DisplayName:      firstNonEmptyTrimmed(cfg.DisplayName, id),
			Provider:         providerID,
			ContextWindow:    cfg.ContextWindow,
			SupportsImages:   cfg.SupportsImages,
			ReasoningEfforts: append([]string(nil), cfg.ReasoningEfforts...),
			DefaultReasoning: cfg.DefaultReasoning,
			Disabled:         cfg.Disabled,
		},
		SupportsTools: cfg.SupportsTools,
	}, nil
}

func parseAPIType(value string) (APIType, error) {
	switch APIType(strings.TrimSpace(value)) {
	case "", APITypeResponses:
		return APITypeResponses, nil
	case APITypeChatCompletions:
		return APITypeChatCompletions, nil
	case APITypeAnthropic:
		return APITypeAnthropic, nil
	default:
		return "", fmt.Errorf("unsupported model type %q; allowed values: %s, %s, %s", value, APITypeResponses, APITypeChatCompletions, APITypeAnthropic)
	}
}

func resolveAPIKey(cfg UserModelConfig, env map[string]string) (apiKey string, apiKeyEnv string, hasAPIKey bool) {
	if key := strings.TrimSpace(cfg.APIKey); strings.HasPrefix(key, "env:") {
		apiKeyEnv = strings.TrimSpace(strings.TrimPrefix(key, "env:"))
		_, hasAPIKey = env[apiKeyEnv]
		return "", apiKeyEnv, hasAPIKey
	} else if key != "" {
		return key, "", true
	}
	apiKeyEnv = strings.TrimSpace(cfg.APIKeyEnv)
	if apiKeyEnv == "" {
		return "", "", false
	}
	_, hasAPIKey = env[apiKeyEnv]
	return "", apiKeyEnv, hasAPIKey
}

func (cfg UserModelConfig) RedactForLog() string {
	parts := []string{
		"id=" + cfg.ID,
		"type=" + cfg.Type,
		"provider=" + cfg.Provider,
		"model=" + cfg.Model,
		"base_url=" + cfg.BaseURL,
	}
	if cfg.APIKey != "" {
		parts = append(parts, "api_key=<redacted>")
	}
	if cfg.APIKeyEnv != "" {
		parts = append(parts, "api_key_env=<redacted>")
	}
	if len(cfg.Headers) > 0 {
		parts = append(parts, "headers="+strings.Join(sortedKeys(cfg.Headers), ","))
	}
	if len(cfg.HeaderEnv) > 0 {
		parts = append(parts, "header_env=<redacted>")
	}
	if cfg.ContextWindow > 0 {
		parts = append(parts, fmt.Sprintf("context_window=%d", cfg.ContextWindow))
	}
	if cfg.Disabled {
		parts = append(parts, "disabled=true")
	}
	return strings.Join(parts, " ")
}

func cloneStringMap(in map[string]string) map[string]string {
	if len(in) == 0 {
		return nil
	}
	out := make(map[string]string, len(in))
	for key, value := range in {
		out[key] = value
	}
	return out
}

func sortedKeys(in map[string]string) []string {
	keys := make([]string, 0, len(in))
	for key := range in {
		keys = append(keys, key)
	}
	sort.Strings(keys)
	return keys
}

func firstNonEmptyTrimmed(values ...string) string {
	for _, value := range values {
		if strings.TrimSpace(value) != "" {
			return strings.TrimSpace(value)
		}
	}
	return ""
}
