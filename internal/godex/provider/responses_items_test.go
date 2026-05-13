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

func TestResponsesItemConversionSerializesInputImages(t *testing.T) {
	items := responseInputItems([]Message{{
		Role:    RoleUser,
		Content: "what is in this image?",
		Images:  []Image{{URL: "data:image/png;base64,abc", Detail: "high"}},
	}})
	data, err := json.Marshal(items)
	if err != nil {
		t.Fatalf("marshal input: %v", err)
	}
	raw := string(data)
	for _, want := range []string{
		`"role":"user"`,
		`"type":"input_text"`,
		`"text":"what is in this image?"`,
		`"type":"input_image"`,
		`"image_url":"data:image/png;base64,abc"`,
		`"detail":"high"`,
	} {
		if !strings.Contains(raw, want) {
			t.Fatalf("input JSON missing %q:\n%s", want, raw)
		}
	}
}

func TestOpenAIItemConversionPreservesOutputShapes(t *testing.T) {
	items := providerItemsFromRaw([]json.RawMessage{
		json.RawMessage(`{"id":"msg_1","type":"message","role":"assistant","phase":"commentary","content":[{"type":"output_text","text":"hello"}]}`),
		json.RawMessage(`{"id":"fc_1","type":"function_call","call_id":"call_1","name":"read_file","arguments":"{\"path\":\"README.md\"}"}`),
		json.RawMessage(`{"type":"function_call_output","call_id":"call_1","output":"contents"}`),
		json.RawMessage(`{"id":"rs_1","type":"reasoning","summary":[{"type":"summary_text","text":"looked around"}]}`),
		json.RawMessage(`{"id":"cmp_1","type":"compaction","encrypted_content":"opaque"}`),
		json.RawMessage(`{"id":"weird_1","type":"new_future_item","payload":true}`),
	})
	if len(items) != 6 {
		t.Fatalf("items = %#v", items)
	}
	if items[0].Kind != ItemMessage || items[0].Role != "assistant" || items[0].Phase != PhaseCommentary || items[0].Text != "hello" {
		t.Fatalf("message item = %#v", items[0])
	}
	if items[1].Kind != ItemFunctionCall || items[1].ToolCallID != "call_1" || items[1].ToolName != "read_file" || !strings.Contains(items[1].Text, "README.md") {
		t.Fatalf("function call item = %#v", items[1])
	}
	if items[2].Kind != ItemFunctionOut || items[2].ToolCallID != "call_1" || items[2].Text != "contents" {
		t.Fatalf("function output item = %#v", items[2])
	}
	if items[3].Kind != ItemReasoning || items[3].Text != "looked around" {
		t.Fatalf("reasoning item = %#v", items[3])
	}
	if items[4].Kind != ItemCompaction || len(items[4].RawJSON) == 0 {
		t.Fatalf("compaction item = %#v", items[4])
	}
	if items[5].Kind != ItemRaw || !strings.Contains(string(items[5].RawJSON), "new_future_item") {
		t.Fatalf("raw item = %#v", items[5])
	}
}

func TestOpenAIItemConversionPreservesToolSearchRoundTripItems(t *testing.T) {
	items := providerItemsFromRaw([]json.RawMessage{
		json.RawMessage(`{"type":"tool_search_call","execution":"server","call_id":null,"status":"completed","arguments":{"paths":["gode"]}}`),
		json.RawMessage(`{"type":"tool_search_output","execution":"server","call_id":null,"status":"completed","tools":[{"type":"namespace","name":"gode","tools":[]}]}`),
	})
	if len(items) != 2 {
		t.Fatalf("items = %#v", items)
	}
	for _, item := range items {
		if item.Kind != ItemRaw || len(item.RawJSON) == 0 {
			t.Fatalf("tool search item should be replayed as raw item: %#v", item)
		}
	}
	input := providerInputItems(items)
	data, err := json.Marshal(input)
	if err != nil {
		t.Fatalf("marshal input: %v", err)
	}
	raw := string(data)
	for _, want := range []string{`"type":"tool_search_call"`, `"type":"tool_search_output"`, `"execution":"server"`} {
		if !strings.Contains(raw, want) {
			t.Fatalf("input JSON missing %q:\n%s", want, raw)
		}
	}
}

func TestFinalAnswerTextFromRawIgnoresCommentary(t *testing.T) {
	text := finalAnswerTextFromRaw([]json.RawMessage{
		json.RawMessage(`{"id":"msg_1","type":"message","role":"assistant","phase":"commentary","content":[{"type":"output_text","text":"I will inspect first."}]}`),
		json.RawMessage(`{"id":"msg_2","type":"message","role":"assistant","phase":"final_answer","content":[{"type":"output_text","text":"Done."}]}`),
	})
	if text != "Done." {
		t.Fatalf("final answer text = %q", text)
	}
}

func TestCrossProviderOpenAISerializationUsesCanonicalFields(t *testing.T) {
	items := providerInputItems([]Item{
		{
			ID:      "anthropic_text",
			Kind:    ItemMessage,
			Role:    "assistant",
			Text:    "I read it.",
			RawJSON: json.RawMessage(`{"type":"text","text":"anthropic raw should not be replayed"}`),
		},
		{
			ID:         "toolu_01",
			Kind:       ItemFunctionCall,
			ToolName:   "read_file",
			ToolCallID: "toolu_01",
			Text:       `{"path":"README.md"}`,
			RawJSON:    json.RawMessage(`{"type":"tool_use","id":"toolu_01","name":"read_file","input":{"path":"README.md"}}`),
		},
		{
			ID:         "out_01",
			Kind:       ItemFunctionOut,
			ToolCallID: "toolu_01",
			Text:       "contents",
			RawJSON:    json.RawMessage(`{"type":"tool_result","tool_use_id":"toolu_01","content":"contents"}`),
		},
	})
	data, err := json.Marshal(items)
	if err != nil {
		t.Fatalf("marshal input: %v", err)
	}
	raw := string(data)
	for _, want := range []string{
		`"role":"assistant"`,
		`"type":"function_call"`,
		`"type":"function_call_output"`,
		`"call_id":"toolu_01"`,
		`"name":"read_file"`,
		`"output":"contents"`,
	} {
		if !strings.Contains(raw, want) {
			t.Fatalf("input JSON missing %q:\n%s", want, raw)
		}
	}
	for _, notWant := range []string{"tool_use", "tool_result", "anthropic raw should not be replayed"} {
		if strings.Contains(raw, notWant) {
			t.Fatalf("OpenAI input should use canonical fields, found %q in:\n%s", notWant, raw)
		}
	}
}
