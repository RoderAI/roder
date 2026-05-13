package provider

import (
	"encoding/json"
	"strings"
	"testing"
)

func TestResponsesItemConversionPreservesFunctionCallsAndOutputs(t *testing.T) {
	items := responseInputItems([]Message{
		{Role: RoleUser, Content: "hello"},
		{Role: RoleAssistant, ToolCallID: "call_123", ToolName: "read_file", ToolArguments: `{"path":"README.md"}`},
		{Role: RoleTool, ToolCallID: "call_123", Content: "tool result"},
	})
	data, err := json.Marshal(items)
	if err != nil {
		t.Fatalf("marshal input: %v", err)
	}
	raw := string(data)
	for _, want := range []string{
		`"role":"user"`,
		`"type":"function_call"`,
		`"type":"function_call_output"`,
		`"call_id":"call_123"`,
		`"name":"read_file"`,
		`"output":"tool result"`,
	} {
		if !strings.Contains(raw, want) {
			t.Fatalf("input JSON missing %q:\n%s", want, raw)
		}
	}
}

func TestResponsesItemConversionPreservesRawCompactionItems(t *testing.T) {
	items := responseInputItems([]Message{{
		RawJSON: json.RawMessage(`{"type":"compaction","encrypted_content":"opaque","id":"cmp_123"}`),
	}})
	data, err := json.Marshal(items)
	if err != nil {
		t.Fatalf("marshal input: %v", err)
	}
	for _, want := range []string{`"type":"compaction"`, `"encrypted_content":"opaque"`, `"id":"cmp_123"`} {
		if !strings.Contains(string(data), want) {
			t.Fatalf("input JSON missing %q:\n%s", want, data)
		}
	}
}
