package provider

import (
	"encoding/json"
	"strings"
	"testing"
)

func TestGeminiInputFromResponsesItemsToolLoop(t *testing.T) {
	input, err := GeminiInputFromResponsesItems([]Item{
		{ID: "sys", Kind: ItemMessage, Role: "system", Text: "You are gode."},
		{ID: "user", Kind: ItemMessage, Role: "user", Text: "Read README."},
		{ID: "call", Kind: ItemFunctionCall, ToolCallID: "call_1", ToolName: "read_file", Text: `{"path":"README.md"}`, RawJSON: json.RawMessage(`{"functionCall":{"id":"call_1","name":"read_file"},"thoughtSignature":"c2ln"}`)},
		{ID: "out", Kind: ItemFunctionOut, ToolCallID: "call_1", Text: "file text"},
		{ID: "answer", Kind: ItemMessage, Role: "assistant", Text: "Done."},
	}, []ToolSpec{{Name: "read_file", Description: "Read a file", Schema: map[string]any{"type": "object", "required": []any{"path"}}}})
	if err != nil {
		t.Fatalf("convert: %v", err)
	}
	data, err := json.Marshal(input)
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	raw := string(data)
	for _, want := range []string{`"system_instruction":[{"text":"You are gode."}]`, `"role":"user"`, `"role":"model"`, `"function_call":{"id":"call_1","name":"read_file"`, `"path":"README.md"`, `"function_response":{"id":"call_1","name":"read_file"`, `"output":"file text"`, `"function_declarations"`} {
		if !strings.Contains(raw, want) {
			t.Fatalf("Gemini input missing %q:\n%s", want, raw)
		}
	}
}

func TestGeminiInputReplaysSignedFunctionCallThoughtSignature(t *testing.T) {
	input, err := GeminiInputFromResponsesItems([]Item{
		{Kind: ItemMessage, Role: "user", Text: "Read README."},
		{Kind: ItemFunctionCall, ToolCallID: "call_1", ToolName: "read_file", Text: `{"path":"README.md"}`, RawJSON: json.RawMessage(`{"functionCall":{"id":"call_1","name":"read_file"},"thoughtSignature":"c2ln"}`)},
		{Kind: ItemFunctionOut, ToolCallID: "call_1", Text: "file text"},
	}, nil)
	if err != nil {
		t.Fatalf("convert: %v", err)
	}
	if len(input.Contents) < 2 || len(input.Contents[1].Parts) != 1 {
		t.Fatalf("contents = %#v", input.Contents)
	}
	part := input.Contents[1].Parts[0]
	if part.FunctionCall == nil || string(part.ThoughtSignature) != "sig" {
		t.Fatalf("function call part = %#v", part)
	}
	sdkPart := input.Contents[1].SDKContent().Parts[0]
	if sdkPart.FunctionCall == nil || string(sdkPart.ThoughtSignature) != "sig" {
		t.Fatalf("sdk part = %#v", sdkPart)
	}
}

func TestGeminiInputReplaysUnsignedToolLoopWithSyntheticThoughtSignature(t *testing.T) {
	input, err := GeminiInputFromResponsesItems([]Item{
		{Kind: ItemMessage, Role: "user", Text: "Where am I?"},
		{Kind: ItemFunctionCall, ToolCallID: "call_1", ToolName: "shell", Text: `{"command":"pwd"}`},
		{Kind: ItemFunctionOut, ToolCallID: "call_1", Text: "/Users/pz/w/gode\n"},
	}, nil)
	if err != nil {
		t.Fatalf("convert: %v", err)
	}
	if len(input.Contents) != 3 {
		t.Fatalf("contents = %#v", input.Contents)
	}
	call := input.Contents[1].Parts[0]
	if call.FunctionCall == nil || call.FunctionCall.Name != "shell" {
		t.Fatalf("function call part = %#v", call)
	}
	if string(call.ThoughtSignature) != geminiSyntheticThoughtSignature {
		t.Fatalf("thought signature = %q", string(call.ThoughtSignature))
	}
	response := input.Contents[2].Parts[0]
	if response.FunctionResponse == nil || response.FunctionResponse.Name != "shell" || response.FunctionResponse.ID != "call_1" {
		t.Fatalf("function response part = %#v", response)
	}
}

func TestGeminiInputGroupsAdjacentToolOutputs(t *testing.T) {
	input, err := GeminiInputFromResponsesItems([]Item{
		{Kind: ItemFunctionCall, ToolCallID: "a", ToolName: "one", Text: `{}`, RawJSON: json.RawMessage(`{"functionCall":{"id":"a","name":"one"},"thoughtSignature":"c2ln"}`)},
		{Kind: ItemFunctionCall, ToolCallID: "b", ToolName: "two", Text: `{}`},
		{Kind: ItemFunctionOut, ToolCallID: "a", Text: "first"},
		{Kind: ItemFunctionOut, ToolCallID: "b", Text: "second"},
	}, nil)
	if err != nil {
		t.Fatalf("convert: %v", err)
	}
	if len(input.Contents) != 2 {
		t.Fatalf("contents = %#v", input.Contents)
	}
	if input.Contents[1].Role != "user" || len(input.Contents[1].Parts) != 2 {
		t.Fatalf("tool responses not grouped: %#v", input.Contents[1])
	}
}

func TestGeminiInputRejectsRemoteImageURL(t *testing.T) {
	_, err := GeminiInputFromResponsesItems([]Item{{ID: "img", Kind: ItemMessage, Role: "user", Images: []Image{{URL: "https://example.com/image.png"}}}}, nil)
	if err == nil {
		t.Fatal("expected nonportable image error")
	}
	if !strings.Contains(err.Error(), "gemini") || !strings.Contains(err.Error(), "data:image") {
		t.Fatalf("error = %v", err)
	}
}

func TestGeminiInputConvertsDataURLImage(t *testing.T) {
	input, err := GeminiInputFromResponsesItems([]Item{{Kind: ItemMessage, Role: "user", Text: "look", Images: []Image{{URL: "data:image/png;base64,YWJj"}}}}, nil)
	if err != nil {
		t.Fatalf("convert: %v", err)
	}
	if got := string(input.Contents[0].Parts[1].InlineData.Data); got != "abc" {
		t.Fatalf("image data = %q", got)
	}
	if input.Contents[0].Parts[1].InlineData.MIMEType != "image/png" {
		t.Fatalf("mime = %q", input.Contents[0].Parts[1].InlineData.MIMEType)
	}
}

func TestGeminiInputRejectsNonPortableCompaction(t *testing.T) {
	_, err := GeminiInputFromResponsesItems([]Item{{ID: "compact", Kind: ItemCompaction}}, nil)
	if err == nil {
		t.Fatal("expected nonportable compaction error")
	}
	if !strings.Contains(err.Error(), "gemini") || !strings.Contains(err.Error(), "provider-neutral") {
		t.Fatalf("error = %v", err)
	}
}

func TestGeminiInputOmitsRawItems(t *testing.T) {
	input, err := GeminiInputFromResponsesItems([]Item{
		{Kind: ItemMessage, Role: "user", Text: "before"},
		{ID: "raw", Kind: ItemRaw, RawJSON: json.RawMessage(`{"type":"unknown","text":"provider-specific raw"}`)},
		{Kind: ItemMessage, Role: "user", Text: "after"},
	}, nil)
	if err != nil {
		t.Fatalf("convert: %v", err)
	}
	data, err := json.Marshal(input)
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	raw := string(data)
	if strings.Contains(raw, "provider-specific") {
		t.Fatalf("raw item was replayed:\n%s", raw)
	}
	for _, want := range []string{"before", "after"} {
		if !strings.Contains(raw, want) {
			t.Fatalf("input missing %q:\n%s", want, raw)
		}
	}
}

func TestGeminiInputRejectsRawText(t *testing.T) {
	_, err := GeminiInputFromResponsesItems([]Item{{ID: "raw-text", Kind: ItemRaw, Text: "provider-specific text"}}, nil)
	if err == nil {
		t.Fatal("expected nonportable raw error")
	}
	if !strings.Contains(err.Error(), "gemini") || !strings.Contains(err.Error(), "raw provider-specific text") {
		t.Fatalf("error = %v", err)
	}
}
