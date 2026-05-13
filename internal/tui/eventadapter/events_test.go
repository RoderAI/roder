package eventadapter

import (
	"strings"
	"testing"

	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

func TestApplySummarizesReadFileToolOutput(t *testing.T) {
	update := Apply(eventbus.Event{
		Kind: eventbus.KindToolCompleted,
		Payload: map[string]any{
			"tool": "read_file",
			"input": map[string]any{
				"path": "internal/godex/tools/registry.go",
			},
			"text": strings.Repeat("package main\n", 200),
		},
	})

	if len(update.Messages) != 1 {
		t.Fatalf("messages = %#v", update.Messages)
	}
	message := update.Messages[0]
	if message.Role != viewmodel.RoleTool || message.Title != "read_file" {
		t.Fatalf("message = %#v", message)
	}
	if message.Body != "read internal/godex/tools/registry.go" {
		t.Fatalf("body = %q", message.Body)
	}
	if strings.Contains(message.Body, "package main") {
		t.Fatalf("read_file summary leaked contents:\n%s", message.Body)
	}
}

func TestApplyStateEventsDoNotRenderTranscriptRows(t *testing.T) {
	events := []eventbus.Event{
		{Kind: eventbus.KindPermissionResponded, Payload: map[string]any{"decision": "allow"}},
		{Kind: eventbus.KindMCPStateChanged, Payload: map[string]any{"server": "fs", "state": "connected"}},
		{Kind: eventbus.KindLSPStateChanged, Payload: map[string]any{"server": "gopls", "state": "connected"}},
		{Kind: KindHookResult, Payload: map[string]any{"hook": "guard", "decision": "allow"}},
		{Kind: KindSessionUpdate, Payload: map[string]any{"title": "worktree"}},
		{Kind: KindModelChanged, Payload: map[string]any{"model": "gpt-5.5"}},
	}
	for _, ev := range events {
		update := Apply(ev)
		if len(update.Messages) != 0 {
			t.Fatalf("%s rendered transcript rows: %#v", ev.Kind, update.Messages)
		}
		if !update.HasStatus || strings.TrimSpace(update.Status) == "" {
			t.Fatalf("%s did not expose a useful status: %#v", ev.Kind, update)
		}
	}
}
