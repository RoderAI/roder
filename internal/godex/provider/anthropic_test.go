package provider

import (
	"encoding/json"
	"strings"
	"testing"
)

func TestAnthropicParamsFromCanonicalItems(t *testing.T) {
	anthropicProvider := NewAnthropicWithConfig(AnthropicConfig{Model: "claude-sonnet-4-6", MaxTokens: 1234})
	params, err := anthropicProvider.messageParams(Request{
		PreviousResponseID: "ignored",
		InputItems: []Item{
			{ID: "sys", Kind: ItemMessage, Role: "system", Text: "You are gode."},
			{ID: "user", Kind: ItemMessage, Role: "user", Text: "Read the file."},
		},
		Tools: []ToolSpec{{
			Name:        "read_file",
			Description: "Read a file from the workspace.",
			Schema: map[string]any{
				"type": "object",
				"properties": map[string]any{
					"path": map[string]any{"type": "string"},
				},
				"required": []any{"path"},
			},
		}},
	})
	if err != nil {
		t.Fatalf("params: %v", err)
	}
	data, err := json.Marshal(params)
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	raw := string(data)
	for _, want := range []string{
		`"model":"claude-sonnet-4-6"`,
		`"max_tokens":1234`,
		`"system":[{"text":"You are gode.","type":"text"}]`,
		`"messages":[{"content":[{"text":"Read the file.","type":"text"}],"role":"user"}]`,
		`"name":"read_file"`,
		`"required":["path"]`,
	} {
		if !strings.Contains(raw, want) {
			t.Fatalf("params JSON missing %q:\n%s", want, raw)
		}
	}
	if strings.Contains(raw, "previous_response_id") {
		t.Fatalf("Anthropic params should ignore previous response IDs:\n%s", raw)
	}
}

func TestAnthropicParamsUsesLegacyMessagesDuringTransition(t *testing.T) {
	anthropicProvider := NewAnthropic("")
	params, err := anthropicProvider.messageParams(Request{
		Messages: []Message{{Role: RoleUser, Content: "hello"}},
	})
	if err != nil {
		t.Fatalf("params: %v", err)
	}
	data, err := json.Marshal(params.Messages)
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	if !strings.Contains(string(data), "hello") {
		t.Fatalf("messages JSON = %s", data)
	}
}
