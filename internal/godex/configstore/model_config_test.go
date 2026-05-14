package configstore

import (
	"path/filepath"
	"testing"

	"github.com/pandelisz/gode/internal/godex/provider"
)

func TestLoadUserModelTOMLIntoConfig(t *testing.T) {
	dataDir := filepath.Join(t.TempDir(), "data")
	writeFile(t, filepath.Join(dataDir, "config.toml"), `
[model.deepseek-chat]
type = "chat_completions"
provider = "deepseek"
model = "deepseek-chat"
base_url = "https://api.deepseek.example/v1"
api_key_env = "DEEPSEEK_API_KEY"
context_window = 128000
`)

	loaded, err := Load(LoadOptions{DataDir: dataDir, Env: []string{"HOME=" + t.TempDir()}})
	if err != nil {
		t.Fatalf("load: %v", err)
	}
	cfg, ok := loaded.Config.UserModels["deepseek-chat"]
	if !ok {
		t.Fatalf("missing user model: %#v", loaded.Config.UserModels)
	}
	if cfg.Type != string(provider.APITypeChatCompletions) || cfg.Provider != "deepseek" || cfg.Model != "deepseek-chat" {
		t.Fatalf("model identity = %#v", cfg)
	}
	if cfg.BaseURL != "https://api.deepseek.example/v1" || cfg.APIKeyEnv != "DEEPSEEK_API_KEY" || cfg.ContextWindow != 128000 {
		t.Fatalf("model endpoint = %#v", cfg)
	}
}

func TestLoadUserModelInvalidTypeFails(t *testing.T) {
	dataDir := filepath.Join(t.TempDir(), "data")
	writeFile(t, filepath.Join(dataDir, "config.toml"), `
[model.bad]
type = "legacy"
provider = "custom"
`)

	_, err := Load(LoadOptions{DataDir: dataDir, Env: []string{"HOME=" + t.TempDir()}})
	if err == nil {
		t.Fatal("expected invalid model type error")
	}
}
