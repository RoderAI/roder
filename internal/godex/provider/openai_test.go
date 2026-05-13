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
