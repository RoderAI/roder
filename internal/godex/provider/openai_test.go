package provider

import (
	"testing"

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
