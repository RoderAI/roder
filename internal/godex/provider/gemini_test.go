package provider

import (
	"context"
	"encoding/json"
	"errors"
	"reflect"
	"strings"
	"testing"

	"google.golang.org/genai"
)

func TestGeminiGenerateContentRequestToolsSchemaAndReasoning(t *testing.T) {
	prov := newGeminiWithClient(GeminiConfig{Model: "gemini-3.1-pro-preview", Reasoning: "high"}, fakeGeminiClient{}, genai.BackendGeminiAPI, "", "")
	contents, config, err := prov.generateContentRequest(Request{
		Instructions: "Follow repo instructions.",
		InputItems:   []Item{{Kind: ItemMessage, Role: "system", Text: "Be helpful."}, {Kind: ItemMessage, Role: "user", Text: "Use a tool."}},
		Tools: []ToolSpec{{Name: "read_file", Description: "Read a file", Schema: map[string]any{
			"type":       "object",
			"properties": map[string]any{"path": map[string]any{"type": "string", "description": "file path", "enum": []any{"README.md"}}},
			"required":   []any{"path"},
		}}},
	})
	if err != nil {
		t.Fatalf("request: %v", err)
	}
	if len(contents) != 1 || contents[0].Role != "user" || contents[0].Parts[0].Text != "Use a tool." {
		t.Fatalf("contents = %#v", contents)
	}
	if config.SystemInstruction == nil || len(config.SystemInstruction.Parts) != 2 {
		t.Fatalf("system = %#v", config.SystemInstruction)
	}
	if got := []string{config.SystemInstruction.Parts[0].Text, config.SystemInstruction.Parts[1].Text}; !reflect.DeepEqual(got, []string{"Follow repo instructions.", "Be helpful."}) {
		t.Fatalf("system instruction parts = %#v", got)
	}
	if config.ThinkingConfig == nil || config.ThinkingConfig.ThinkingLevel != genai.ThinkingLevelHigh {
		t.Fatalf("thinking = %#v", config.ThinkingConfig)
	}
	if len(config.Tools) != 1 || len(config.Tools[0].FunctionDeclarations) != 1 {
		t.Fatalf("tools = %#v", config.Tools)
	}
	decl := config.Tools[0].FunctionDeclarations[0]
	if decl.Name != "read_file" || decl.Description != "Read a file" {
		t.Fatalf("decl = %#v", decl)
	}
	data, _ := json.Marshal(decl.ParametersJsonSchema)
	for _, want := range []string{`"properties"`, `"required"`, `"enum"`, `"README.md"`} {
		if !strings.Contains(string(data), want) {
			t.Fatalf("schema missing %q: %s", want, data)
		}
	}
	if config.ToolConfig == nil || config.ToolConfig.FunctionCallingConfig.Mode != genai.FunctionCallingConfigModeAuto {
		t.Fatalf("tool config = %#v", config.ToolConfig)
	}
}

func TestGeminiResponseFormatJSONSchema(t *testing.T) {
	config, err := geminiGenerateConfig(Request{ResponseFormat: `{"type":"json_schema","schema":{"type":"object","properties":{"ok":{"type":"boolean"}},"required":["ok"]}}`}, "none")
	if err != nil {
		t.Fatalf("config: %v", err)
	}
	if config.ResponseMIMEType != "application/json" {
		t.Fatalf("mime = %q", config.ResponseMIMEType)
	}
	data, _ := json.Marshal(config.ResponseJsonSchema)
	if !strings.Contains(string(data), `"ok"`) {
		t.Fatalf("schema = %s", data)
	}
}

func TestGeminiResponseFormatJSONModeWithoutSchema(t *testing.T) {
	config, err := geminiGenerateConfig(Request{ResponseFormat: `{"type":"json_object"}`}, "none")
	if err != nil {
		t.Fatalf("config: %v", err)
	}
	if config.ResponseMIMEType != "application/json" {
		t.Fatalf("mime = %q", config.ResponseMIMEType)
	}
	if config.ResponseJsonSchema != nil {
		t.Fatalf("schema should be omitted for json mode: %#v", config.ResponseJsonSchema)
	}
}

func TestGeminiThinkingConfig(t *testing.T) {
	cases := map[string]genai.ThinkingLevel{"minimal": genai.ThinkingLevelMinimal, "low": genai.ThinkingLevelLow, "medium": genai.ThinkingLevelMedium, "high": genai.ThinkingLevelHigh}
	for effort, want := range cases {
		got, err := geminiThinkingConfig(effort)
		if err != nil {
			t.Fatalf("%s: %v", effort, err)
		}
		if got == nil || got.ThinkingLevel != want {
			t.Fatalf("%s = %#v", effort, got)
		}
	}
	if got, err := geminiThinkingConfig("none"); err != nil || got != nil {
		t.Fatalf("none = %#v err=%v", got, err)
	}
	xhigh, err := geminiThinkingConfig("xhigh")
	if err != nil {
		t.Fatalf("xhigh: %v", err)
	}
	if xhigh.ThinkingLevel != genai.ThinkingLevelHigh || xhigh.ThinkingBudget == nil || *xhigh.ThinkingBudget == 0 {
		t.Fatalf("xhigh = %#v", xhigh)
	}
}

func TestGeminiBackendConfig(t *testing.T) {
	backend, err := geminiBackend("enterprise")
	if err != nil {
		t.Fatalf("backend: %v", err)
	}
	if backend != genai.BackendEnterprise {
		t.Fatalf("backend = %v", backend)
	}
	if _, err := geminiBackend("bad"); err == nil {
		t.Fatal("expected bad backend error")
	}
}

func TestNewGeminiWithConfigRedactsInitError(t *testing.T) {
	t.Setenv("GEMINI_API_TOKEN", "secret-token")
	_, err := NewGeminiWithConfig(context.Background(), GeminiConfig{
		Model:   "gemini-3.1-pro-preview",
		APIKey:  "secret-token",
		Headers: map[string]string{"X-API-Key": "header-secret"},
		Project: "project-secret",
	})
	if err == nil {
		t.Fatal("expected mutually exclusive auth error")
	}
	for _, leaked := range []string{"secret-token", "header-secret", "project-secret"} {
		if strings.Contains(err.Error(), leaked) {
			t.Fatalf("error leaked %q: %v", leaked, err)
		}
	}
}

func TestGeminiStreamErrorRedactsConfiguredSecrets(t *testing.T) {
	prov := newGeminiWithClient(
		GeminiConfig{
			Model:    "gemini-3.1-pro-preview",
			APIKey:   "literal-secret",
			Headers:  map[string]string{"X-API-Key": "header-secret"},
			Project:  "project-secret",
			Location: "location-secret",
		},
		fakeGeminiClient{err: errors.New("literal-secret header-secret project-secret location-secret failed")},
		genai.BackendGeminiAPI,
		"project-secret",
		"location-secret",
	)
	_, errs := prov.Stream(context.Background(), Request{InputItems: []Item{{Kind: ItemMessage, Role: "user", Text: "hi"}}})
	err := <-errs
	if err == nil {
		t.Fatal("expected stream error")
	}
	for _, leaked := range []string{"literal-secret", "header-secret", "project-secret", "location-secret"} {
		if strings.Contains(err.Error(), leaked) {
			t.Fatalf("stream error leaked %q: %v", leaked, err)
		}
	}
	if !strings.Contains(err.Error(), "<redacted>") {
		t.Fatalf("error leaked secret: %v", err)
	}
}

type fakeGeminiClient struct {
	responses []*genai.GenerateContentResponse
	err       error
}

func (f fakeGeminiClient) GenerateContentStream(ctx context.Context, model string, contents []*genai.Content, config *genai.GenerateContentConfig) geminiResponseIterator {
	return fakeGeminiIterator{responses: f.responses, err: f.err}
}

type fakeGeminiIterator struct {
	responses []*genai.GenerateContentResponse
	err       error
}

func (f fakeGeminiIterator) ForEach(yield func(*genai.GenerateContentResponse, error) bool) {
	for _, resp := range f.responses {
		if !yield(resp, nil) {
			return
		}
	}
	if f.err != nil {
		yield(nil, f.err)
	}
}

func TestGeminiSDKToolsPreserveSchemaObject(t *testing.T) {
	schema := map[string]any{"type": "object", "properties": map[string]any{"path": map[string]any{"type": "string"}}}
	tools := geminiSDKTools([]ToolSpec{{Name: "read_file", Schema: schema}})
	if len(tools) != 1 || len(tools[0].FunctionDeclarations) != 1 {
		t.Fatalf("tools = %#v", tools)
	}
	if !reflect.DeepEqual(tools[0].FunctionDeclarations[0].ParametersJsonSchema, schema) {
		t.Fatalf("schema changed = %#v", tools[0].FunctionDeclarations[0].ParametersJsonSchema)
	}
}
