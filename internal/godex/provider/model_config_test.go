package provider

import (
	"strings"
	"testing"
)

func TestResolveUserModelChatCompletionsConfig(t *testing.T) {
	cfg := UserModelConfig{
		Type:          "chat_completions",
		Provider:      "deepseek",
		Model:         "deepseek-chat",
		BaseURL:       "https://api.deepseek.example/v1",
		APIKeyEnv:     "DEEPSEEK_API_KEY",
		ContextWindow: 128000,
	}
	resolved, err := ResolveUserModel("deepseek-chat", cfg, map[string]string{"DEEPSEEK_API_KEY": "secret-key"})
	if err != nil {
		t.Fatalf("resolve: %v", err)
	}
	if resolved.ID != "deepseek-chat" || resolved.APIType != APITypeChatCompletions || resolved.ProviderID != "deepseek" {
		t.Fatalf("resolved identity = %#v", resolved)
	}
	if resolved.UpstreamModel != "deepseek-chat" || resolved.BaseURL != "https://api.deepseek.example/v1" || resolved.APIKeyEnv != "DEEPSEEK_API_KEY" || !resolved.HasAPIKey {
		t.Fatalf("resolved endpoint/auth = %#v", resolved)
	}
	if resolved.Metadata.ContextWindow != 128000 {
		t.Fatalf("context window = %d", resolved.Metadata.ContextWindow)
	}
}

func TestResolveUserModelPreservesEditTool(t *testing.T) {
	resolved, err := ResolveUserModel("router-gpt", UserModelConfig{
		Type:     string(APITypeResponses),
		Provider: "router",
		Model:    "gpt-5.5",
		EditTool: "patch",
	}, nil)
	if err != nil {
		t.Fatalf("resolve: %v", err)
	}
	if resolved.EditTool != "patch" || resolved.Metadata.EditTool != "patch" {
		t.Fatalf("edit tool = %#v metadata=%#v", resolved.EditTool, resolved.Metadata.EditTool)
	}
}

func TestResolveUserModelRejectsInvalidEditTool(t *testing.T) {
	_, err := ResolveUserModel("bad-edit-tool", UserModelConfig{EditTool: "rewrite"}, nil)
	if err == nil {
		t.Fatal("expected invalid edit_tool error")
	}
	if !strings.Contains(err.Error(), "patch, edit") {
		t.Fatalf("error should list allowed edit tools, got %v", err)
	}
}

func TestResolveUserModelEnvAPIKeyDoesNotMutateConfig(t *testing.T) {
	cfg := UserModelConfig{
		Type:     "chat_completions",
		Provider: "deepseek",
		APIKey:   "env:DEEPSEEK_API_KEY",
	}
	resolved, err := ResolveUserModel("deepseek-chat", cfg, map[string]string{"DEEPSEEK_API_KEY": "secret-key"})
	if err != nil {
		t.Fatalf("resolve: %v", err)
	}
	if resolved.APIKey != "" || resolved.APIKeyEnv != "DEEPSEEK_API_KEY" || !resolved.HasAPIKey {
		t.Fatalf("resolved auth = %#v", resolved)
	}
	if cfg.APIKey != "env:DEEPSEEK_API_KEY" {
		t.Fatalf("config mutated API key = %q", cfg.APIKey)
	}
}

func TestUserModelRedactForLogHidesLiteralSecrets(t *testing.T) {
	cfg := UserModelConfig{
		Type:     "responses",
		Provider: "openai-compatible",
		Model:    "my-model",
		APIKey:   "literal-secret",
		Headers:  map[string]string{"X-API-Key": "header-secret"},
	}
	redacted := cfg.RedactForLog()
	if !strings.Contains(redacted, "api_key=<redacted>") {
		t.Fatalf("redacted = %#v", redacted)
	}
	if strings.Contains(redacted, "literal-secret") || strings.Contains(redacted, "header-secret") {
		t.Fatalf("redacted string leaked secret: %s", redacted)
	}
}

func TestResolveUserModelInvalidTypeListsAllowedValues(t *testing.T) {
	_, err := ResolveUserModel("bad", UserModelConfig{Type: "old_api", Provider: "x"}, nil)
	if err == nil {
		t.Fatal("expected invalid type error")
	}
	for _, want := range []string{"responses", "chat_completions", "anthropic"} {
		if !strings.Contains(err.Error(), want) {
			t.Fatalf("error should list %q, got %v", want, err)
		}
	}
}

func TestResolveUserModelDefaultsMissingModelToTableKey(t *testing.T) {
	resolved, err := ResolveUserModel("local-id", UserModelConfig{Type: "anthropic", Provider: "router"}, nil)
	if err != nil {
		t.Fatalf("resolve: %v", err)
	}
	if resolved.UpstreamModel != "local-id" {
		t.Fatalf("upstream model = %q", resolved.UpstreamModel)
	}
}
