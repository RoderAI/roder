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
			"tool":         "read_file",
			"tool_call_id": "call_1",
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
	if message.Key != "tool:call_1" {
		t.Fatalf("message key = %q", message.Key)
	}
	if message.Body != "succeeded\nread internal/godex/tools/registry.go" {
		t.Fatalf("body = %q", message.Body)
	}
	if strings.Contains(message.Body, "package main") {
		t.Fatalf("read_file summary leaked contents:\n%s", message.Body)
	}
}

func TestApplySummarizesShellToolWithCommandNotOutput(t *testing.T) {
	update := Apply(eventbus.Event{
		Kind: eventbus.KindToolCompleted,
		Payload: map[string]any{
			"tool":         "shell",
			"tool_call_id": "call_1",
			"input": map[string]any{
				"command": "go test ./internal/tui -count=1",
			},
			"text": "ok\nPASS\n",
		},
	})

	if len(update.Messages) != 1 {
		t.Fatalf("messages = %#v", update.Messages)
	}
	message := update.Messages[0]
	if message.Body != "succeeded\ngo test ./internal/tui -count=1" {
		t.Fatalf("body = %q", message.Body)
	}
	if strings.Contains(message.Body, "PASS") {
		t.Fatalf("shell summary leaked output:\n%s", message.Body)
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

func TestApplyToolRequestedShowsRunningTimelineRow(t *testing.T) {
	update := Apply(eventbus.Event{
		Kind: eventbus.KindToolRequested,
		Payload: map[string]any{
			"tool":         "apply_patch",
			"tool_call_id": "call_1",
		},
	})

	if len(update.Messages) != 1 {
		t.Fatalf("messages = %#v", update.Messages)
	}
	message := update.Messages[0]
	if message.Key != "tool:call_1" || message.Role != viewmodel.RoleTool || message.Title != "apply_patch" {
		t.Fatalf("message = %#v", message)
	}
	if message.Body != "requested" {
		t.Fatalf("body = %q", message.Body)
	}
	if !update.HasStatus || update.Status != "tool requested: apply_patch" {
		t.Fatalf("status = %#v", update)
	}
}

func TestApplyToolRequestedIncludesInputSummaryWhilePending(t *testing.T) {
	update := Apply(eventbus.Event{
		Kind: eventbus.KindToolRequested,
		Payload: map[string]any{
			"tool":         "shell",
			"tool_call_id": "call_1",
			"input": map[string]any{
				"command": "sleep 8; printf done",
			},
		},
	})

	if len(update.Messages) != 1 {
		t.Fatalf("messages = %#v", update.Messages)
	}
	message := update.Messages[0]
	if message.Key != "tool:call_1" || message.Role != viewmodel.RoleTool || message.Title != "shell" {
		t.Fatalf("message = %#v", message)
	}
	if message.Body != "requested\nsleep 8; printf done" {
		t.Fatalf("body = %q", message.Body)
	}
}

func TestApplyContextTokensUpdatedExposesUsageWithoutTranscriptRows(t *testing.T) {
	update := Apply(eventbus.Event{
		Kind: eventbus.KindContextTokensUpdated,
		Payload: map[string]any{
			"tokens":         1250,
			"context_window": 10000,
			"percent":        12.5,
		},
	})

	if len(update.Messages) != 0 {
		t.Fatalf("context token event rendered transcript rows: %#v", update.Messages)
	}
	if !update.HasContextTokens || update.ContextUsedPercent != 12.5 {
		t.Fatalf("context update = %#v", update)
	}
}

func TestApplyContextTokensUpdatedIgnoresResponseUsageIncrement(t *testing.T) {
	update := Apply(eventbus.Event{
		Kind: eventbus.KindContextTokensUpdated,
		Payload: map[string]any{
			"tokens":          42,
			"context_window":  10000,
			"percent":         0.42,
			"usage_increment": true,
		},
	})

	if update.HasContextTokens {
		t.Fatalf("response usage increment should not update context meter: %#v", update)
	}
}

func TestApplyContextCompactionRepairedShowsRetryStatus(t *testing.T) {
	update := Apply(eventbus.Event{Kind: eventbus.KindContextCompactionRepaired})
	if !update.HasStatus || update.Status != "repaired context; retrying compact" {
		t.Fatalf("repair update = %#v", update)
	}
}

func TestApplyPermissionRequestRendersToolMetadata(t *testing.T) {
	update := Apply(eventbus.Event{
		Kind: eventbus.KindPermissionRequested,
		Payload: map[string]any{
			"tool":   "write_file",
			"action": "write",
			"path":   "README.md",
		},
	})
	if len(update.Messages) != 1 {
		t.Fatalf("messages = %#v", update.Messages)
	}
	message := update.Messages[0]
	if message.Role != viewmodel.RoleTool || message.Title != "write_file" {
		t.Fatalf("message = %#v", message)
	}
	for _, want := range []string{"permission requested", "action: write", "path: README.md"} {
		if !strings.Contains(message.Body, want) {
			t.Fatalf("body missing %q:\n%s", want, message.Body)
		}
	}
}

func TestApplyToolCompletedIncludesHookMetadata(t *testing.T) {
	update := Apply(eventbus.Event{
		Kind: eventbus.KindToolCompleted,
		Payload: map[string]any{
			"tool":          "shell",
			"text":          "ok",
			"hook_decision": "allow",
			"hook_warnings": []string{"checked policy"},
		},
	})
	if len(update.Messages) != 1 {
		t.Fatalf("messages = %#v", update.Messages)
	}
	body := update.Messages[0].Body
	for _, want := range []string{"ok", "hook: allow", "hook warnings: checked policy"} {
		if !strings.Contains(body, want) {
			t.Fatalf("body missing %q:\n%s", want, body)
		}
	}
}

func TestApplyToolFailedKeepsToolTimelineRole(t *testing.T) {
	update := Apply(eventbus.Event{
		Kind: eventbus.KindToolFailed,
		Payload: map[string]any{
			"tool":         "apply_patch",
			"tool_call_id": "call_1",
			"error":        "status 128",
		},
	})
	if len(update.Messages) != 1 {
		t.Fatalf("messages = %#v", update.Messages)
	}
	message := update.Messages[0]
	if message.Role != viewmodel.RoleTool || message.Title != "apply_patch" {
		t.Fatalf("message = %#v", message)
	}
	if message.Body != "failed: status 128" {
		t.Fatalf("body = %q", message.Body)
	}
}
