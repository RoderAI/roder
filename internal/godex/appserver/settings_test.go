package appserver

import (
	"context"
	"path/filepath"
	"testing"
	"time"

	"github.com/pandelisz/gode/internal/godex"
)

func TestSettingsGetAndUpdateRuntimeSettings(t *testing.T) {
	ctx := context.Background()
	dataDir := t.TempDir()
	app, err := godex.New(ctx, godex.Config{
		Workspace:   filepath.Join(t.TempDir(), "workspace"),
		DataDir:     dataDir,
		Provider:    "mock",
		Model:       "mock",
		Reasoning:   "none",
		AutoApprove: false,
	})
	if err != nil {
		t.Fatalf("new app: %v", err)
	}
	defer app.Close(ctx)

	var messages []Message
	conn := New(app, Options{Version: "test"}).NewConnection(func(_ context.Context, msg Message) error {
		messages = append(messages, msg)
		return nil
	})
	initializeTestConnection(t, conn)

	sendJSONRequest(t, conn, map[string]any{"id": 2, "method": "settings/get"})
	cfg := objectField(t, responseResult(t, messages, 2), "config")
	if cfg["model"] != "mock" || cfg["reasoning"] != "none" {
		t.Fatalf("settings/get config = %#v", cfg)
	}

	sendJSONRequest(t, conn, map[string]any{
		"id":     3,
		"method": "settings/update",
		"params": map[string]any{
			"defaultModel":          "gpt-5.4-mini",
			"defaultReasoning":      godex.ReasoningHigh,
			"fastMode":              true,
			"autoApprove":           true,
			"markdownRendering":     false,
			"disableAutoCompaction": true,
			"autoCompactTokenLimit": 12345,
		},
	})
	updated := objectField(t, responseResult(t, messages, 3), "config")
	if updated["model"] != "gpt-5.4-mini" || updated["reasoning"] != godex.ReasoningHigh {
		t.Fatalf("updated model/reasoning = %#v", updated)
	}
	if updated["fastMode"] != true || updated["autoApprove"] != true || updated["markdownRendering"] != false {
		t.Fatalf("updated toggles = %#v", updated)
	}
	if updated["disableAutoCompaction"] != true || updated["autoCompactTokenLimit"] != float64(12345) {
		t.Fatalf("updated compaction = %#v", updated)
	}
	waitFor(t, time.Second, func() bool {
		return hasNotification(messages, "settings/changed")
	})

	settings, err := godex.LoadSettings(dataDir)
	if err != nil {
		t.Fatalf("load settings: %v", err)
	}
	if settings.DefaultModel != "gpt-5.4-mini" || settings.DefaultReasoning != godex.ReasoningHigh {
		t.Fatalf("persisted settings = %#v", settings)
	}
	if !settings.FastMode || !settings.AutoApprove || settings.MarkdownRendering {
		t.Fatalf("persisted toggles = %#v", settings)
	}
}

func TestSettingsUpdateRejectsInvalidReasoning(t *testing.T) {
	ctx := context.Background()
	app, err := godex.New(ctx, godex.Config{
		Workspace: filepath.Join(t.TempDir(), "workspace"),
		DataDir:   t.TempDir(),
		Provider:  "mock",
		Model:     "mock",
		Reasoning: "none",
	})
	if err != nil {
		t.Fatalf("new app: %v", err)
	}
	defer app.Close(ctx)

	var messages []Message
	conn := New(app, Options{Version: "test"}).NewConnection(func(_ context.Context, msg Message) error {
		messages = append(messages, msg)
		return nil
	})
	initializeTestConnection(t, conn)

	sendJSONRequest(t, conn, map[string]any{
		"id":     4,
		"method": "settings/update",
		"params": map[string]any{
			"defaultModel":     "mock",
			"defaultReasoning": "xhigh",
		},
	})
	if got := responseErrorMessage(t, messages, 4); got == "" {
		t.Fatalf("expected invalid reasoning error")
	}
}
