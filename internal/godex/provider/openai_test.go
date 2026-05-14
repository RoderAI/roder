package provider

import (
	"encoding/json"
	"io"
	"net/http"
	"strings"
	"testing"

	openai "github.com/openai/openai-go/v3"
	"github.com/openai/openai-go/v3/responses"
)

func TestOpenAIResponseParamsUsesPriorityServiceTierForFastMode(t *testing.T) {
	openaiProvider := NewOpenAIWithConfig(OpenAIConfig{
		Model:       "gpt-5.5",
		Reasoning:   "medium",
		ServiceTier: "priority",
	})

	params := openaiProvider.responseParams(Request{Messages: []Message{{Role: RoleUser, Content: "hello"}}})
	if params.ServiceTier != responses.ResponseNewParamsServiceTierPriority {
		t.Fatalf("service tier = %q, want priority", params.ServiceTier)
	}
}

func TestOpenAIResponseParamsOmitsServiceTierByDefault(t *testing.T) {
	openaiProvider := NewOpenAI("gpt-5.5", "medium")

	params := openaiProvider.responseParams(Request{Messages: []Message{{Role: RoleUser, Content: "hello"}}})
	if params.ServiceTier != "" {
		t.Fatalf("service tier = %q, want empty", params.ServiceTier)
	}
}

func TestOpenAIResponseParamsIncludesInstructions(t *testing.T) {
	openaiProvider := NewOpenAI("gpt-5.5", "medium")

	params := openaiProvider.responseParams(Request{
		Instructions: "You are gode.",
		Messages:     []Message{{Role: RoleUser, Content: "hello"}},
	})
	if !params.Instructions.Valid() {
		t.Fatal("instructions should be set")
	}
	if params.Instructions.Value != "You are gode." {
		t.Fatalf("instructions = %q", params.Instructions.Value)
	}
	if !params.Store.Valid() {
		t.Fatal("store should be set")
	}
	if params.Store.Value {
		t.Fatal("store should be false")
	}
	if params.Reasoning.Summary == "" {
		t.Fatal("reasoning summary should be requested")
	}
}

func TestOpenAIResponseParamsIncludesPromptCacheKey(t *testing.T) {
	openaiProvider := NewOpenAI("gpt-5.5", "medium")

	params := openaiProvider.responseParams(Request{
		PromptCacheKey: "gode:openai:gpt-5-5:abc123",
		Messages:       []Message{{Role: RoleUser, Content: "hello"}},
	})
	if !params.PromptCacheKey.Valid() || params.PromptCacheKey.Value != "gode:openai:gpt-5-5:abc123" {
		t.Fatalf("prompt cache key = %#v", params.PromptCacheKey)
	}
}

func TestOpenAICompactParamsIncludesPromptCacheKey(t *testing.T) {
	openaiProvider := NewOpenAI("gpt-5.5", "medium")

	params := openaiProvider.compactParams(CompactRequest{
		Model:          "gpt-5.5",
		PromptCacheKey: "gode:openai:gpt-5-5:abc123",
		Messages:       []Message{{Role: RoleUser, Content: "hello"}},
	})
	if !params.PromptCacheKey.Valid() || params.PromptCacheKey.Value != "gode:openai:gpt-5-5:abc123" {
		t.Fatalf("prompt cache key = %#v", params.PromptCacheKey)
	}
}

func TestOpenAICompactParamsTruncatesHugeFunctionCallOutputs(t *testing.T) {
	openaiProvider := NewOpenAI("gpt-5.5", "medium")
	huge := strings.Repeat("a", compactFunctionOutputLimitBytes+4096) + "TAIL_SENTINEL"

	params := openaiProvider.compactParams(CompactRequest{
		Model: "gpt-5.5",
		Messages: []Message{
			{Role: RoleAssistant, ToolCallID: "call_123", ToolName: "shell", ToolArguments: `{"command":"cat huge.log"}`},
			{Role: RoleTool, ToolCallID: "call_123", Content: huge},
		},
	})

	output := compactFunctionOutputFromParams(t, params.Input)
	if len(output) > compactFunctionOutputLimitBytes {
		t.Fatalf("compacted output length = %d, want <= %d", len(output), compactFunctionOutputLimitBytes)
	}
	for _, want := range []string{"gode truncated this tool output for compaction", "TAIL_SENTINEL"} {
		if !strings.Contains(output, want) {
			t.Fatalf("compacted output missing %q", want)
		}
	}
	if strings.Contains(output, strings.Repeat("a", compactFunctionOutputLimitBytes)) {
		t.Fatal("compacted output still contains the full oversized body")
	}
}

func TestOpenAICompactParamsTruncatesHugeRawFunctionCallOutputs(t *testing.T) {
	openaiProvider := NewOpenAI("gpt-5.5", "medium")
	huge := strings.Repeat("b", compactFunctionOutputLimitBytes+4096) + "RAW_TAIL_SENTINEL"
	rawOutput, err := json.Marshal(huge)
	if err != nil {
		t.Fatalf("marshal raw output: %v", err)
	}

	params := openaiProvider.compactParams(CompactRequest{
		Model: "gpt-5.5",
		Messages: []Message{{
			RawJSON: json.RawMessage(`{"type":"function_call_output","call_id":"call_raw","output":` + string(rawOutput) + `}`),
		}},
	})

	output := compactFunctionOutputFromParams(t, params.Input)
	if len(output) > compactFunctionOutputLimitBytes {
		t.Fatalf("compacted raw output length = %d, want <= %d", len(output), compactFunctionOutputLimitBytes)
	}
	for _, want := range []string{"gode truncated this tool output for compaction", "RAW_TAIL_SENTINEL"} {
		if !strings.Contains(output, want) {
			t.Fatalf("compacted raw output missing %q", want)
		}
	}
}

func TestOpenAIResponseParamsSupportsLocalItemsPreviousResponseIDAndStoreFalse(t *testing.T) {
	openaiProvider := NewOpenAI("gpt-5.5", "medium")

	params := openaiProvider.responseParams(Request{
		PreviousResponseID: "resp_123",
		InputItems: []Item{{
			ID:      "msg_1",
			Kind:    ItemMessage,
			Role:    "user",
			Text:    "hello from item",
			RawJSON: json.RawMessage(`{"id":"msg_1","type":"message","role":"user","content":[{"type":"input_text","text":"hello from item"}]}`),
		}},
	})
	if !params.PreviousResponseID.Valid() || params.PreviousResponseID.Value != "resp_123" {
		t.Fatalf("previous response id = %#v", params.PreviousResponseID)
	}
	if !params.Store.Valid() || params.Store.Value {
		t.Fatalf("store should default false: %#v", params.Store)
	}
	data, err := json.Marshal(params.Input)
	if err != nil {
		t.Fatalf("marshal input: %v", err)
	}
	if !strings.Contains(string(data), `"hello from item"`) {
		t.Fatalf("input items not used:\n%s", data)
	}
}

func TestOpenAITokenUsageUsesResponseUsageFields(t *testing.T) {
	usage := openAITokenUsage(responses.ResponseUsage{
		InputTokens:  11,
		OutputTokens: 7,
		TotalTokens:  18,
	})
	if usage.InputTokens != 11 || usage.OutputTokens != 7 || usage.Total() != 18 {
		t.Fatalf("usage = %#v", usage)
	}
}

func TestOpenAIResponseParamsIncludesResponseFormat(t *testing.T) {
	openaiProvider := NewOpenAI("gpt-5.5", "medium")

	params := openaiProvider.responseParams(Request{
		ResponseFormat: `{"type":"json_object"}`,
		Messages:       []Message{{Role: RoleUser, Content: "hello"}},
	})
	data, err := json.Marshal(params.Text)
	if err != nil {
		t.Fatalf("marshal text config: %v", err)
	}
	if !strings.Contains(string(data), `"type":"json_object"`) {
		t.Fatalf("text config missing response format:\n%s", data)
	}
}

func TestOpenAIResponseParamsUsesInputItemList(t *testing.T) {
	openaiProvider := NewOpenAI("gpt-5.5", "medium")

	params := openaiProvider.responseParams(Request{
		Messages: []Message{
			{Role: RoleUser, Content: "hello"},
			{Role: RoleAssistant, Content: "hi"},
			{Role: RoleAssistant, ToolCallID: "call_123", ToolName: "read_file", ToolArguments: `{"path":"README.md"}`},
			{Role: RoleTool, ToolCallID: "call_123", Content: "tool result"},
		},
	})
	if params.Input.OfString.Valid() {
		t.Fatalf("input should not use string form: %#v", params.Input.OfString)
	}
	if len(params.Input.OfInputItemList) != 4 {
		t.Fatalf("input list length = %d", len(params.Input.OfInputItemList))
	}
	data, err := json.Marshal(params.Input)
	if err != nil {
		t.Fatalf("marshal input: %v", err)
	}
	raw := string(data)
	for _, want := range []string{
		`"role":"user"`,
		`"role":"assistant"`,
		`"type":"function_call"`,
		`"type":"function_call_output"`,
		`"call_id":"call_123"`,
		`"name":"read_file"`,
		`"arguments":"{\"path\":\"README.md\"}"`,
		`"output":"tool result"`,
	} {
		if !strings.Contains(raw, want) {
			t.Fatalf("input JSON missing %q:\n%s", want, raw)
		}
	}
}

func TestOpenAIResponseParamsPreservesAssistantPhase(t *testing.T) {
	openaiProvider := NewOpenAI("gpt-5.5", "medium")

	params := openaiProvider.responseParams(Request{
		Messages: []Message{{
			Role:    RoleAssistant,
			Phase:   PhaseCommentary,
			Content: "I will inspect the workspace first.",
		}},
	})
	data, err := json.Marshal(params.Input)
	if err != nil {
		t.Fatalf("marshal input: %v", err)
	}
	raw := string(data)
	for _, want := range []string{`"role":"assistant"`, `"phase":"commentary"`, "I will inspect"} {
		if !strings.Contains(raw, want) {
			t.Fatalf("input JSON missing %q:\n%s", want, raw)
		}
	}
}

func TestOpenAIResponseParamsPreservesRawCompactionItems(t *testing.T) {
	openaiProvider := NewOpenAI("gpt-5.5", "medium")
	rawItem := json.RawMessage(`{"type":"compaction","encrypted_content":"opaque","id":"cmp_123"}`)

	params := openaiProvider.responseParams(Request{
		Messages: []Message{{RawJSON: rawItem}},
	})
	data, err := json.Marshal(params.Input)
	if err != nil {
		t.Fatalf("marshal input: %v", err)
	}
	for _, want := range []string{`"type":"compaction"`, `"encrypted_content":"opaque"`, `"id":"cmp_123"`} {
		if !strings.Contains(string(data), want) {
			t.Fatalf("input JSON missing %q:\n%s", want, data)
		}
	}
}

func TestOpenAIResponseParamsKeepsCoreToolsEagerAndDefersUnknownTools(t *testing.T) {
	openaiProvider := NewOpenAI("gpt-5.5", "medium")

	params := openaiProvider.responseParams(Request{
		Messages: []Message{{Role: RoleUser, Content: "hello"}},
		Tools: []ToolSpec{{
			Name:        "read_file",
			Description: "Read a file",
			Schema:      map[string]any{"type": "object", "properties": map[string]any{}, "required": []string{}},
		}, {
			Name:        "shell",
			Description: "Run a shell command",
			Schema:      map[string]any{"type": "object", "properties": map[string]any{}, "required": []string{}},
		}, {
			Name:        "mcp_helper_echo",
			Description: "Remote MCP echo",
			Schema:      map[string]any{"type": "object", "properties": map[string]any{}, "required": []string{}},
		}},
	})
	data, err := json.Marshal(params.Tools)
	if err != nil {
		t.Fatalf("marshal tools: %v", err)
	}
	raw := string(data)
	for _, want := range []string{
		`"type":"namespace"`,
		`"name":"gode"`,
		`"type":"tool_search"`,
		`"execution":"server"`,
		`"name":"read_file"`,
		`"name":"shell"`,
		`"name":"mcp_helper_echo"`,
		`"defer_loading":true`,
	} {
		if !strings.Contains(raw, want) {
			t.Fatalf("tools JSON missing %q:\n%s", want, raw)
		}
	}
	if got := strings.Count(raw, `"defer_loading":true`); got != 1 {
		t.Fatalf("deferred tool count = %d, want only the unknown tool deferred:\n%s", got, raw)
	}
}

func TestOpenAIResponseParamsOmitsToolSearchWhenAllToolsAreEager(t *testing.T) {
	openaiProvider := NewOpenAI("gpt-5.5", "medium")

	params := openaiProvider.responseParams(Request{
		Messages: []Message{{Role: RoleUser, Content: "hello"}},
		Tools: []ToolSpec{{
			Name:        "read_file",
			Description: "Read a file",
			Schema:      map[string]any{"type": "object", "properties": map[string]any{}, "required": []string{}},
		}, {
			Name:        "apply_patch",
			Description: "Apply a patch",
			Schema:      map[string]any{"type": "object", "properties": map[string]any{}, "required": []string{}},
		}, {
			Name:        "shell",
			Description: "Run a shell command",
			Schema:      map[string]any{"type": "object", "properties": map[string]any{}, "required": []string{}},
		}},
	})
	data, err := json.Marshal(params.Tools)
	if err != nil {
		t.Fatalf("marshal tools: %v", err)
	}
	raw := string(data)
	for _, unwanted := range []string{`"type":"tool_search"`, `"defer_loading":true`} {
		if strings.Contains(raw, unwanted) {
			t.Fatalf("eager-only tools JSON should not contain %q:\n%s", unwanted, raw)
		}
	}
	for _, want := range []string{`"type":"namespace"`, `"name":"read_file"`, `"name":"apply_patch"`, `"name":"shell"`} {
		if !strings.Contains(raw, want) {
			t.Fatalf("tools JSON missing %q:\n%s", want, raw)
		}
	}
}

func TestOpenAIResponseParamsIncludesServerSideCompaction(t *testing.T) {
	openaiProvider := NewOpenAI("gpt-5.5", "medium")

	params := openaiProvider.responseParams(Request{
		Messages: []Message{{Role: RoleUser, Content: "hello"}},
		Compaction: CompactionOptions{
			Enabled:          true,
			Model:            "gpt-5.5",
			ContextWindow:    1050000,
			CompactThreshold: 800000,
		},
	})
	data, err := json.Marshal(params)
	if err != nil {
		t.Fatalf("marshal params: %v", err)
	}
	raw := string(data)
	for _, want := range []string{
		`"context_management":[{"type":"compaction","compact_threshold":800000}]`,
		`"store":false`,
	} {
		if !strings.Contains(raw, want) {
			t.Fatalf("params JSON missing %q:\n%s", want, raw)
		}
	}
	if strings.Contains(raw, `"truncation"`) {
		t.Fatalf("params JSON should omit truncation for Codex backend compatibility:\n%s", raw)
	}
}

func TestOpenAIResponseParamsOmitsCompactionWhenDisabled(t *testing.T) {
	openaiProvider := NewOpenAI("gpt-5.5", "medium")

	params := openaiProvider.responseParams(Request{
		Messages: []Message{{Role: RoleUser, Content: "hello"}},
		Compaction: CompactionOptions{
			Model:            "gpt-future",
			ContextWindow:    272000,
			CompactThreshold: 217600,
		},
	})
	data, err := json.Marshal(params)
	if err != nil {
		t.Fatalf("marshal params: %v", err)
	}
	raw := string(data)
	if strings.Contains(raw, "context_management") {
		t.Fatalf("params JSON should omit compaction:\n%s", raw)
	}
	if strings.Contains(raw, `"truncation"`) {
		t.Fatalf("params JSON should omit truncation:\n%s", raw)
	}
}

func TestOpenAIResponseParamsNormalizesNullRequiredToolSchema(t *testing.T) {
	openaiProvider := NewOpenAI("gpt-5.5", "medium")

	params := openaiProvider.responseParams(Request{
		Messages: []Message{{Role: RoleUser, Content: "hello"}},
		Tools: []ToolSpec{{
			Name:        "git_diff",
			Description: "Show git diff",
			Schema: map[string]any{
				"type":       "object",
				"properties": map[string]any{},
				"required":   []string(nil),
			},
		}},
	})
	data, err := json.Marshal(params.Tools)
	if err != nil {
		t.Fatalf("marshal tools: %v", err)
	}
	raw := string(data)
	if strings.Contains(raw, `"required":null`) {
		t.Fatalf("tool schema should not contain required:null:\n%s", raw)
	}
	if !strings.Contains(raw, `"required":[]`) {
		t.Fatalf("tool schema should contain required array:\n%s", raw)
	}
}

func TestOpenAIResponseParamsAllowsParallelToolCalls(t *testing.T) {
	openaiProvider := NewOpenAI("gpt-5.5", "medium")

	params := openaiProvider.responseParams(Request{
		Messages: []Message{{Role: RoleUser, Content: "hello"}},
		Tools: []ToolSpec{{
			Name:        "read_file",
			Description: "Read a file",
			Schema:      map[string]any{"type": "object", "properties": map[string]any{}, "required": []string{}},
		}},
	})
	if !params.ParallelToolCalls.Valid() || !params.ParallelToolCalls.Value {
		t.Fatalf("parallel tool calls = %#v", params.ParallelToolCalls)
	}
}

func TestOpenAIStreamErrorIncludesHTTPDebugDetails(t *testing.T) {
	openaiProvider := NewOpenAIWithConfig(OpenAIConfig{
		Model:       "gpt-5.5",
		Reasoning:   "medium",
		ServiceTier: "priority",
	})
	req, err := http.NewRequest(http.MethodPost, "https://chatgpt.com/backend-api/codex/responses", nil)
	if err != nil {
		t.Fatalf("request: %v", err)
	}
	apiErr := &openai.Error{
		StatusCode: http.StatusBadRequest,
		Request:    req,
		Response: &http.Response{
			StatusCode: http.StatusBadRequest,
			Header:     http.Header{"X-Request-Id": []string{"req_123"}},
			Body:       io.NopCloser(strings.NewReader(`{"detail":"unsupported model for codex subscription"}`)),
		},
		Message: "bad request",
		Type:    "invalid_request_error",
		Param:   "model",
	}

	detail := openaiProvider.formatStreamError(apiErr, Request{
		Messages: []Message{{Role: RoleUser, Content: "hello"}},
		Tools:    []ToolSpec{{Name: "read_file"}},
	})
	for _, want := range []string{
		"request: POST https://chatgpt.com/backend-api/codex/responses",
		"status: 400 Bad Request",
		"x-request-id: req_123",
		"error_type: invalid_request_error",
		"error_param: model",
		"response_body:",
		"unsupported model for codex subscription",
		"model: gpt-5.5",
		"service_tier: priority",
		"tool_names: read_file",
	} {
		if !strings.Contains(detail, want) {
			t.Fatalf("debug detail missing %q:\n%s", want, detail)
		}
	}
}

func compactFunctionOutputFromParams(t *testing.T, input responses.ResponseCompactParamsInputUnion) string {
	t.Helper()
	data, err := json.Marshal(input)
	if err != nil {
		t.Fatalf("marshal compact input: %v", err)
	}
	var items []map[string]any
	if err := json.Unmarshal(data, &items); err != nil {
		t.Fatalf("unmarshal compact input %s: %v", data, err)
	}
	for _, item := range items {
		if item["type"] != "function_call_output" {
			continue
		}
		output, ok := item["output"].(string)
		if !ok {
			t.Fatalf("function_call_output output is %T, want string", item["output"])
		}
		return output
	}
	t.Fatalf("compact input has no function_call_output: %s", data)
	return ""
}
