package provider

import (
	"encoding/json"
	"strings"
	"testing"
)

func TestChatInputFromResponsesItemsMessages(t *testing.T) {
	input, err := ChatInputFromResponsesItems([]Item{
		{ID: "sys", Kind: ItemMessage, Role: "system", Text: "You are concise."},
		{ID: "user", Kind: ItemMessage, Role: "user", Text: "Hello"},
		{ID: "assistant", Kind: ItemMessage, Role: "assistant", Text: "Hi."},
	}, nil)
	if err != nil {
		t.Fatalf("convert: %v", err)
	}
	if got := []string{input.Messages[0].Role, input.Messages[1].Role, input.Messages[2].Role}; got[0] != "system" || got[1] != "user" || got[2] != "assistant" {
		t.Fatalf("roles = %#v", got)
	}
	if input.Messages[0].Content != "You are concise." || input.Messages[1].Content != "Hello" || input.Messages[2].Content != "Hi." {
		t.Fatalf("messages = %#v", input.Messages)
	}
}

func TestChatInputFromResponsesItemsFunctionCall(t *testing.T) {
	input, err := ChatInputFromResponsesItems([]Item{
		{ID: "a1", Kind: ItemMessage, Role: "assistant", Text: "I'll inspect that."},
		{ID: "c1", Kind: ItemFunctionCall, ToolName: "read_file", ToolCallID: "call_1", Text: `{"path":"README.md"}`},
	}, nil)
	if err != nil {
		t.Fatalf("convert: %v", err)
	}
	if len(input.Messages) != 1 {
		t.Fatalf("messages = %#v", input.Messages)
	}
	msg := input.Messages[0]
	if msg.Role != "assistant" || msg.Content != "I'll inspect that." || len(msg.ToolCalls) != 1 {
		t.Fatalf("assistant message = %#v", msg)
	}
	call := msg.ToolCalls[0]
	if call.ID != "call_1" || call.Type != "function" || call.Function.Name != "read_file" || call.Function.Arguments != `{"path":"README.md"}` {
		t.Fatalf("tool call = %#v", call)
	}
}

func TestChatInputFromResponsesItemsToolOutputsPreserveOrder(t *testing.T) {
	input, err := ChatInputFromResponsesItems([]Item{
		{ID: "o1", Kind: ItemFunctionOut, ToolCallID: "call_1", Text: "one"},
		{ID: "o2", Kind: ItemFunctionOut, ToolCallID: "call_2", Text: "two"},
	}, nil)
	if err != nil {
		t.Fatalf("convert: %v", err)
	}
	if len(input.Messages) != 2 {
		t.Fatalf("messages = %#v", input.Messages)
	}
	if input.Messages[0].Role != "tool" || input.Messages[0].ToolCallID != "call_1" || input.Messages[0].Content != "one" {
		t.Fatalf("first tool output = %#v", input.Messages[0])
	}
	if input.Messages[1].Role != "tool" || input.Messages[1].ToolCallID != "call_2" || input.Messages[1].Content != "two" {
		t.Fatalf("second tool output = %#v", input.Messages[1])
	}
}

func TestChatInputFromResponsesItemsReasoningRawAndCompaction(t *testing.T) {
	input, err := ChatInputFromResponsesItems([]Item{
		{ID: "r1", Kind: ItemReasoning, Text: "hidden"},
		{ID: "raw1", Kind: ItemRaw, RawJSON: json.RawMessage(`{"type":"future"}`)},
		{ID: "cmp1", Kind: ItemCompaction, Text: "Earlier context summary..."},
	}, nil)
	if err != nil {
		t.Fatalf("convert: %v", err)
	}
	if len(input.Messages) != 1 || input.Messages[0].Role != "user" || input.Messages[0].Content != "Earlier context summary..." {
		t.Fatalf("messages = %#v", input.Messages)
	}
	if len(input.DebugEvents) != 1 || !strings.Contains(input.DebugEvents[0], "raw1") {
		t.Fatalf("debug events = %#v", input.DebugEvents)
	}
}

func TestChatToolsFromToolSpecs(t *testing.T) {
	input, err := ChatInputFromResponsesItems(nil, []ToolSpec{{
		Name:        "read_file",
		Description: "Read a file",
		Schema: map[string]any{
			"type": "object",
			"properties": map[string]any{
				"path": map[string]any{"type": "string"},
			},
		},
	}})
	if err != nil {
		t.Fatalf("convert: %v", err)
	}
	if len(input.Tools) != 1 || input.Tools[0].Type != "function" || input.Tools[0].Function.Name != "read_file" {
		t.Fatalf("tools = %#v", input.Tools)
	}
	data, err := json.Marshal(input.Tools[0])
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	for _, want := range []string{`"type":"function"`, `"name":"read_file"`, `"parameters":{"properties":{"path":{"type":"string"}},"type":"object"}`} {
		if !strings.Contains(string(data), want) {
			t.Fatalf("tool JSON missing %q:\n%s", want, data)
		}
	}
}
