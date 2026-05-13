package provider

import (
	"encoding/json"
	"errors"
	"strings"
	"testing"
)

func TestAnthropicInputFromResponsesItemsSystemAndUserText(t *testing.T) {
	input, err := AnthropicInputFromResponsesItems([]Item{
		{ID: "i1", Kind: ItemMessage, Role: "system", Text: "You are concise."},
		{ID: "i2", Kind: ItemMessage, Role: "user", Text: "Hello"},
	}, nil)
	if err != nil {
		t.Fatalf("convert: %v", err)
	}
	assertAnthropicJSON(t, input, `"system":[{"type":"text","text":"You are concise."}]`)
	assertAnthropicJSON(t, input, `"messages":[{"role":"user","content":[{"type":"text","text":"Hello"}]}]`)
}

func TestAnthropicInputFromResponsesItemsAssistantToolUse(t *testing.T) {
	input, err := AnthropicInputFromResponsesItems([]Item{
		{ID: "a1", Kind: ItemMessage, Role: "assistant", Text: "I'll inspect that."},
		{ID: "c1", Kind: ItemFunctionCall, ToolName: "read_file", ToolCallID: "toolu_01", RawJSON: json.RawMessage(`{"path":"README.md"}`)},
	}, nil)
	if err != nil {
		t.Fatalf("convert: %v", err)
	}
	assertAnthropicJSON(t, input, `"role":"assistant"`)
	assertAnthropicJSON(t, input, `"type":"tool_use"`)
	assertAnthropicJSON(t, input, `"id":"toolu_01"`)
	assertAnthropicJSON(t, input, `"name":"read_file"`)
	assertAnthropicJSON(t, input, `"input":{"path":"README.md"}`)
}

func TestAnthropicInputFromResponsesItemsToolResultsBeforeUserText(t *testing.T) {
	input, err := AnthropicInputFromResponsesItems([]Item{
		{ID: "o1", Kind: ItemFunctionOut, ToolCallID: "toolu_01", Text: "file contents"},
		{ID: "u1", Kind: ItemMessage, Role: "user", Text: "Now summarize it."},
	}, nil)
	if err != nil {
		t.Fatalf("convert: %v", err)
	}
	if len(input.Messages) != 1 || input.Messages[0].Role != "user" || len(input.Messages[0].Content) != 2 {
		t.Fatalf("messages = %#v", input.Messages)
	}
	if input.Messages[0].Content[0].Type != "tool_result" || input.Messages[0].Content[1].Type != "text" {
		t.Fatalf("content order = %#v", input.Messages[0].Content)
	}
}

func TestAnthropicInputFromResponsesItemsGroupsParallelToolResults(t *testing.T) {
	input, err := AnthropicInputFromResponsesItems([]Item{
		{ID: "o1", Kind: ItemFunctionOut, ToolCallID: "toolu_01", Text: "one"},
		{ID: "o2", Kind: ItemFunctionOut, ToolCallID: "toolu_02", Text: "two"},
	}, nil)
	if err != nil {
		t.Fatalf("convert: %v", err)
	}
	if len(input.Messages) != 1 || len(input.Messages[0].Content) != 2 {
		t.Fatalf("messages = %#v", input.Messages)
	}
}

func TestAnthropicInputFromResponsesItemsReasoningRawAndCompactionPortability(t *testing.T) {
	input, err := AnthropicInputFromResponsesItems([]Item{
		{ID: "r1", Kind: ItemReasoning, Text: "hidden"},
		{ID: "raw1", Kind: ItemRaw, RawJSON: json.RawMessage(`{"type":"future"}`)},
		{ID: "cmp1", Kind: ItemCompaction, Text: "Earlier context summary..."},
	}, nil)
	if err != nil {
		t.Fatalf("convert: %v", err)
	}
	if len(input.Messages) != 1 || input.Messages[0].Content[0].Text != "Earlier context summary..." {
		t.Fatalf("messages = %#v", input.Messages)
	}
	if len(input.DebugEvents) != 1 || !strings.Contains(input.DebugEvents[0], "raw1") {
		t.Fatalf("debug events = %#v", input.DebugEvents)
	}

	_, err = AnthropicInputFromResponsesItems([]Item{{ID: "cmp_raw", Kind: ItemCompaction, RawJSON: json.RawMessage(`{"type":"compaction","encrypted_content":"opaque"}`)}}, nil)
	var portable NonPortableItemError
	if !errors.As(err, &portable) || !strings.Contains(err.Error(), "cannot replay compaction item cmp_raw with anthropic") {
		t.Fatalf("error = %v", err)
	}
}

func TestAnthropicInputFromResponsesItemsConvertsTools(t *testing.T) {
	input, err := AnthropicInputFromResponsesItems(nil, []ToolSpec{{
		Name:        "read_file",
		Description: "Read a file",
		Schema:      map[string]any{"type": "object"},
	}})
	if err != nil {
		t.Fatalf("convert: %v", err)
	}
	if len(input.Tools) != 1 || input.Tools[0].Name != "read_file" || input.Tools[0].InputSchema["type"] != "object" {
		t.Fatalf("tools = %#v", input.Tools)
	}
}

func TestAnthropicToolsConvertsSchema(t *testing.T) {
	tools, err := anthropicTools([]ToolSpec{{
		Name:        "read_file",
		Description: "Read a file from the workspace.",
		Schema: map[string]any{
			"type": "object",
			"properties": map[string]any{
				"path": map[string]any{"type": "string"},
			},
			"required": []any{"path"},
		},
	}})
	if err != nil {
		t.Fatalf("convert tools: %v", err)
	}
	if len(tools) != 1 {
		t.Fatalf("tools = %#v", tools)
	}
	data, err := json.Marshal(tools[0])
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	for _, want := range []string{
		`"name":"read_file"`,
		`"description":"Read a file from the workspace."`,
		`"input_schema":{"properties":{"path":{"type":"string"}},"required":["path"],"type":"object"}`,
	} {
		if !strings.Contains(string(data), want) {
			t.Fatalf("tool JSON missing %q:\n%s", want, data)
		}
	}
}

func TestAnthropicToolsRejectsInvalidName(t *testing.T) {
	_, err := anthropicTools([]ToolSpec{{Name: "bad name"}})
	if err == nil || !strings.Contains(err.Error(), `tools[0].name "bad name"`) {
		t.Fatalf("error = %v", err)
	}
}

func TestAnthropicToolsDefaultsNilSchema(t *testing.T) {
	tools, err := anthropicTools([]ToolSpec{{Name: "shell"}})
	if err != nil {
		t.Fatalf("convert tools: %v", err)
	}
	if len(tools) != 1 || tools[0].InputSchema["type"] != "object" {
		t.Fatalf("tools = %#v", tools)
	}
	properties, ok := tools[0].InputSchema["properties"].(map[string]any)
	if !ok || len(properties) != 0 {
		t.Fatalf("properties = %#v", tools[0].InputSchema["properties"])
	}
}

func assertAnthropicJSON(t *testing.T, input AnthropicInput, want string) {
	t.Helper()
	data, err := json.Marshal(input)
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	if !strings.Contains(string(data), want) {
		t.Fatalf("JSON missing %q:\n%s", want, data)
	}
}
