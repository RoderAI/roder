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
	APITypeGemini          APIType = "gemini"
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
	EditTool           string            `json:"edit_tool,omitempty" toml:"edit_tool,omitempty"`
	SupportsImages     bool              `json:"supports_images,omitempty" toml:"supports_images,omitempty"`
	SupportsTools      *bool             `json:"supports_tools,omitempty" toml:"supports_tools,omitempty"`
	SupportsCompaction bool              `json:"supports_compaction,omitempty" toml:"supports_compaction,omitempty"`
	Backend            string            `json:"backend,omitempty" toml:"backend,omitempty"`
	Project            string            `json:"project,omitempty" toml:"project,omitempty"`
	ProjectEnv         string            `json:"project_env,omitempty" toml:"project_env,omitempty"`
	Location           string            `json:"location,omitempty" toml:"location,omitempty"`
	LocationEnv        string            `json:"location_env,omitempty" toml:"location_env,omitempty"`
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
	EditTool      string
	Metadata      ModelMetadata
	SupportsTools *bool
	Backend       string
	Project       string
	ProjectEnv    string
	Location      string
	LocationEnv   string
}

var DefaultGeminiEnvAliases = []string{"GEMINI_API_TOKEN", "GEMINI_API_KEY", "GOOGLE_API_KEY", "GOOGLE_GENAI_API_KEY", "GOOGLE_AI_API_KEY"}

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
	if apiType == APITypeGemini && apiKey == "" && apiKeyEnv == "" {
		apiKeyEnv, hasAPIKey = resolveAPIKeyFromEnvAliases(env, DefaultGeminiEnvAliases)
	}
	editTool, err := parseEditTool(cfg.EditTool)
	if err != nil {
		return ResolvedModel{}, err
	}
	return ResolvedModel{
		ID:            id,
		UpstreamModel: upstreamModel,
		APIType:       apiType,
		ProviderID:    providerID,
		BaseURL:       strings.TrimSpace(cfg.BaseURL),
		APIKey:        apiKey,
		APIKeyEnv:     apiKeyEnv,
		HasAPIKey:     hasAPIKey,
		Headers:       resolveHeaders(cfg, env),
		HeaderEnv:     cloneStringMap(cfg.HeaderEnv),
		EditTool:      editTool,
		Metadata: ModelMetadata{
			ID:               id,
			DisplayName:      firstNonEmptyTrimmed(cfg.DisplayName, id),
			Provider:         providerID,
			ContextWindow:    cfg.ContextWindow,
			SupportsImages:   cfg.SupportsImages,
			ReasoningEfforts: append([]string(nil), cfg.ReasoningEfforts...),
			DefaultReasoning: cfg.DefaultReasoning,
			EditTool:         editTool,
			Disabled:         cfg.Disabled,
		},
		SupportsTools: cfg.SupportsTools,
		Backend:       strings.TrimSpace(cfg.Backend),
		Project:       strings.TrimSpace(cfg.Project),
		ProjectEnv:    strings.TrimSpace(cfg.ProjectEnv),
		Location:      strings.TrimSpace(cfg.Location),
		LocationEnv:   strings.TrimSpace(cfg.LocationEnv),
	}, nil
}

func parseEditTool(value string) (string, error) {
	switch strings.TrimSpace(value) {
	case "", "patch", "edit":
		return strings.TrimSpace(value), nil
	default:
		return "", fmt.Errorf("unsupported edit_tool %q; allowed values: patch, edit", value)
	}
}

func resolveHeaders(cfg UserModelConfig, env map[string]string) map[string]string {
	headers := cloneStringMap(cfg.Headers)
	for header, envKey := range cfg.HeaderEnv {
		header = strings.TrimSpace(header)
		envKey = strings.TrimSpace(envKey)
		if header == "" || envKey == "" {
			continue
		}
		if headers == nil {
			headers = map[string]string{}
		}
		headers[header] = env[envKey]
	}
	return headers
}

func parseAPIType(value string) (APIType, error) {
	switch APIType(strings.TrimSpace(value)) {
	case "", APITypeResponses:
		return APITypeResponses, nil
	case APITypeChatCompletions:
		return APITypeChatCompletions, nil
	case APITypeAnthropic:
		return APITypeAnthropic, nil
	case APITypeGemini:
		return APITypeGemini, nil
	default:
		return "", fmt.Errorf("unsupported model type %q; allowed values: %s, %s, %s, %s", value, APITypeResponses, APITypeChatCompletions, APITypeAnthropic, APITypeGemini)
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

func resolveAPIKeyFromEnvAliases(env map[string]string, aliases []string) (string, bool) {
	for _, alias := range aliases {
		alias = strings.TrimSpace(alias)
		if alias == "" {
			continue
		}
		if _, ok := env[alias]; ok {
			return alias, true
		}
	}
	return "", false
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
	if cfg.Backend != "" {
		parts = append(parts, "backend="+cfg.Backend)
	}
	if cfg.Project != "" {
		parts = append(parts, "project=<redacted>")
	}
	if cfg.ProjectEnv != "" {
		parts = append(parts, "project_env=<redacted>")
	}
	if cfg.Location != "" {
		parts = append(parts, "location=<redacted>")
	}
	if cfg.LocationEnv != "" {
		parts = append(parts, "location_env=<redacted>")
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
