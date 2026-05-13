package godex

import (
	"testing"
	"time"

	"github.com/pandelisz/gode/internal/godex/codexauth"
)

func TestUsesCodexAuthForGPTModelsWhenSignedIn(t *testing.T) {
	dataDir := t.TempDir()
	if err := (codexauth.Store{DataDir: dataDir}).Save(codexauth.Tokens{
		Access:  "access",
		Refresh: "refresh",
		Expires: time.Now().Add(time.Hour).UnixMilli(),
	}); err != nil {
		t.Fatalf("save auth: %v", err)
	}

	if !usesCodexAuth(Config{DataDir: dataDir, Provider: "openai", Model: "gpt-5.4-mini"}) {
		t.Fatal("expected signed-in GPT model to use codex auth")
	}
	if usesCodexAuth(Config{DataDir: dataDir, Provider: "openai", Model: "o4-mini"}) {
		t.Fatal("non-GPT model should not use codex auth")
	}
}
