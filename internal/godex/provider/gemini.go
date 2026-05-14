package provider

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"os"
	"strings"

	"google.golang.org/genai"
)

const (
	GeminiBackendAPI        = "gemini_api"
	GeminiBackendEnterprise = "enterprise"
)

type GeminiConfig struct {
	Model       string
	Reasoning   string
	Backend     string
	BaseURL     string
	APIKey      string
	Headers     map[string]string
	HTTPClient  *http.Client
	Project     string
	ProjectEnv  string
	Location    string
	LocationEnv string
}

type Gemini struct {
	client    geminiClient
	model     string
	reasoning string
	backend   string
	baseURL   string
	apiKey    string
	headers   map[string]string
	project   string
	location  string
}

type geminiClient interface {
	GenerateContentStream(context.Context, string, []*genai.Content, *genai.GenerateContentConfig) geminiResponseIterator
}

type geminiResponseIterator interface {
	ForEach(func(*genai.GenerateContentResponse, error) bool)
}

type genaiModelsClient struct {
	models *genai.Models
}

type genaiIterator struct {
	seq func(func(*genai.GenerateContentResponse, error) bool)
}

func (c genaiModelsClient) GenerateContentStream(ctx context.Context, model string, contents []*genai.Content, config *genai.GenerateContentConfig) geminiResponseIterator {
	return genaiIterator{seq: c.models.GenerateContentStream(ctx, model, contents, config)}
}

func (it genaiIterator) ForEach(yield func(*genai.GenerateContentResponse, error) bool) {
	it.seq(yield)
}

func NewGeminiWithConfig(ctx context.Context, cfg GeminiConfig) (*Gemini, error) {
	if cfg.Model == "" {
		cfg.Model = "gemini-3.1-pro-preview"
	}
	backend, err := geminiBackend(cfg.Backend)
	if err != nil {
		return nil, err
	}
	project := firstNonEmpty(strings.TrimSpace(cfg.Project), os.Getenv(strings.TrimSpace(cfg.ProjectEnv)))
	location := firstNonEmpty(strings.TrimSpace(cfg.Location), os.Getenv(strings.TrimSpace(cfg.LocationEnv)))
	apiKey := cfg.APIKey
	if backend == genai.BackendEnterprise {
		apiKey = ""
	}
	clientConfig := &genai.ClientConfig{
		Backend:    backend,
		APIKey:     apiKey,
		HTTPClient: cfg.HTTPClient,
		Project:    project,
		Location:   location,
	}
	if cfg.BaseURL != "" {
		clientConfig.HTTPOptions.BaseURL = cfg.BaseURL
	}
	if len(cfg.Headers) > 0 {
		clientConfig.HTTPOptions.Headers = http.Header{}
		for key, value := range cfg.Headers {
			if strings.TrimSpace(key) != "" {
				clientConfig.HTTPOptions.Headers.Set(key, value)
			}
		}
	}
	client, err := genai.NewClient(ctx, clientConfig)
	if err != nil {
		return nil, redactedGeminiError("initialize", cfg.Model, backend, err, geminiSensitiveValues(cfg, project, location)...)
	}
	return newGeminiWithClient(cfg, genaiModelsClient{models: client.Models}, backend, project, location), nil
}

func newGeminiWithClient(cfg GeminiConfig, client geminiClient, backend genai.Backend, project string, location string) *Gemini {
	if cfg.Model == "" {
		cfg.Model = "gemini-3.1-pro-preview"
	}
	return &Gemini{
		client:    client,
		model:     cfg.Model,
		reasoning: cfg.Reasoning,
		backend:   geminiBackendName(backend),
		baseURL:   cfg.BaseURL,
		apiKey:    cfg.APIKey,
		headers:   cloneStringMap(cfg.Headers),
		project:   project,
		location:  location,
	}
}

func (g *Gemini) Name() string {
	return "gemini"
}

func (g *Gemini) Stream(ctx context.Context, req Request) (<-chan Event, <-chan error) {
	events := make(chan Event)
	errs := make(chan error, 1)
	go func() {
		defer close(events)
		defer close(errs)

		contents, config, err := g.generateContentRequest(req)
		if err != nil {
			errs <- err
			return
		}
		state := newGeminiStreamState()
		iter := g.client.GenerateContentStream(ctx, g.model, contents, config)
		stopped := false
		iter.ForEach(func(resp *genai.GenerateContentResponse, err error) bool {
			if err != nil {
				errs <- g.redactedError("stream", err)
				stopped = true
				return false
			}
			emitted, err := state.Handle(resp)
			if err != nil {
				errs <- err
				stopped = true
				return false
			}
			for _, ev := range emitted {
				select {
				case <-ctx.Done():
					errs <- ctx.Err()
					stopped = true
					return false
				case events <- ev:
				}
			}
			return true
		})
		if stopped {
			return
		}
		select {
		case <-ctx.Done():
			errs <- ctx.Err()
		case events <- state.CompletedEvent():
		}
	}()
	return events, errs
}

func (g *Gemini) generateContentRequest(req Request) ([]*genai.Content, *genai.GenerateContentConfig, error) {
	items := req.InputItems
	if len(items) == 0 {
		items = chatInputItems(req)
	}
	input, err := GeminiInputFromResponsesItems(items, req.Tools)
	if err != nil {
		return nil, nil, err
	}
	if instructions := strings.TrimSpace(req.Instructions); instructions != "" {
		input.SystemInstruction = append([]GeminiPart{{Text: instructions}}, input.SystemInstruction...)
	}
	contents, system := input.SDKContents()
	config, err := geminiGenerateConfig(req, g.reasoning)
	if err != nil {
		return nil, nil, err
	}
	config.SystemInstruction = system
	config.Tools = geminiSDKTools(req.Tools)
	if len(config.Tools) > 0 {
		config.ToolConfig = &genai.ToolConfig{FunctionCallingConfig: &genai.FunctionCallingConfig{Mode: genai.FunctionCallingConfigModeAuto}}
	}
	return contents, config, nil
}

func geminiGenerateConfig(req Request, reasoning string) (*genai.GenerateContentConfig, error) {
	config := &genai.GenerateContentConfig{}
	if strings.TrimSpace(req.ResponseFormat) != "" {
		if err := applyGeminiResponseFormat(config, req.ResponseFormat); err != nil {
			return nil, err
		}
	}
	thinking, err := geminiThinkingConfig(reasoning)
	if err != nil {
		return nil, err
	}
	config.ThinkingConfig = thinking
	return config, nil
}

func applyGeminiResponseFormat(config *genai.GenerateContentConfig, raw string) error {
	trimmed := strings.TrimSpace(raw)
	if trimmed == "" {
		return nil
	}
	config.ResponseMIMEType = "application/json"
	var object map[string]any
	if err := json.Unmarshal([]byte(trimmed), &object); err != nil {
		return fmt.Errorf("Gemini response format must be valid JSON: %w", err)
	}
	if schema, ok := object["schema"]; ok {
		config.ResponseJsonSchema = schema
		return nil
	}
	if format, ok := object["format"]; ok {
		if formatObject, ok := format.(map[string]any); ok {
			if schema, ok := formatObject["schema"]; ok {
				config.ResponseJsonSchema = schema
				return nil
			}
			if formatObject["type"] == "json_schema" {
				config.ResponseJsonSchema = formatObject
				return nil
			}
		}
		return nil
	}
	if typ, ok := object["type"].(string); ok && typ == "json_schema" {
		config.ResponseJsonSchema = object
	}
	return nil
}

func geminiThinkingConfig(reasoning string) (*genai.ThinkingConfig, error) {
	switch strings.TrimSpace(strings.ToLower(reasoning)) {
	case "", "none":
		return nil, nil
	case "minimal":
		return &genai.ThinkingConfig{ThinkingLevel: genai.ThinkingLevelMinimal}, nil
	case "low":
		return &genai.ThinkingConfig{ThinkingLevel: genai.ThinkingLevelLow}, nil
	case "medium":
		return &genai.ThinkingConfig{ThinkingLevel: genai.ThinkingLevelMedium}, nil
	case "high":
		return &genai.ThinkingConfig{ThinkingLevel: genai.ThinkingLevelHigh}, nil
	case "xhigh":
		budget := int32(32768)
		return &genai.ThinkingConfig{ThinkingLevel: genai.ThinkingLevelHigh, ThinkingBudget: &budget}, nil
	default:
		return nil, fmt.Errorf("unsupported Gemini reasoning effort %q; allowed values: none, minimal, low, medium, high, xhigh", reasoning)
	}
}

func geminiBackend(value string) (genai.Backend, error) {
	switch strings.TrimSpace(strings.ToLower(value)) {
	case "", GeminiBackendAPI:
		return genai.BackendGeminiAPI, nil
	case GeminiBackendEnterprise, "vertex", "vertexai", "vertex_ai":
		return genai.BackendEnterprise, nil
	default:
		return genai.BackendUnspecified, fmt.Errorf("unsupported Gemini backend %q; allowed values: gemini_api, enterprise", value)
	}
}

func geminiBackendName(backend genai.Backend) string {
	switch backend {
	case genai.BackendEnterprise, genai.BackendVertexAI:
		return GeminiBackendEnterprise
	default:
		return GeminiBackendAPI
	}
}

func geminiBackendFromName(value string) genai.Backend {
	backend, _ := geminiBackend(value)
	return backend
}

func (g *Gemini) redactedError(operation string, err error) error {
	return redactedGeminiError(operation, g.model, geminiBackendFromName(g.backend), err, g.sensitiveValues()...)
}

func (g *Gemini) sensitiveValues() []string {
	values := []string{g.apiKey, g.project, g.location}
	for _, value := range g.headers {
		values = append(values, value)
	}
	return values
}

func geminiSensitiveValues(cfg GeminiConfig, project string, location string) []string {
	values := []string{cfg.APIKey, project, location}
	for _, value := range cfg.Headers {
		values = append(values, value)
	}
	return values
}

func redactedGeminiError(operation string, model string, backend genai.Backend, err error, sensitive ...string) error {
	if err == nil {
		return nil
	}
	msg := err.Error()
	for _, value := range sensitive {
		value = strings.TrimSpace(value)
		if value != "" {
			msg = strings.ReplaceAll(msg, value, "<redacted>")
		}
	}
	for _, env := range []string{"GEMINI_API_TOKEN", "GEMINI_API_KEY", "GOOGLE_API_KEY", "GOOGLE_GENAI_API_KEY", "GOOGLE_AI_API_KEY"} {
		if value := os.Getenv(env); value != "" {
			msg = strings.ReplaceAll(msg, value, "<redacted>")
		}
	}
	return &ProviderError{Message: fmt.Sprintf("Gemini %s failed for model %s backend %s: %s", operation, model, geminiBackendName(backend), msg)}
}
