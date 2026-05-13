package message

import (
	"context"
	"testing"
	"time"

	"github.com/pandelisz/gode/internal/godex/eventbus"
)

func TestStoreAppendListBySessionAndRun(t *testing.T) {
	ctx := context.Background()
	dataDir := t.TempDir()
	store := Open(dataDir)
	store.now = fixedClock(time.Date(2026, 5, 13, 10, 0, 0, 0, time.UTC))

	first, err := store.Append(ctx, Message{SessionID: "s1", RunID: "r1", Role: RoleUser, Text: "hello"})
	if err != nil {
		t.Fatalf("append first: %v", err)
	}
	store.now = fixedClock(time.Date(2026, 5, 13, 10, 1, 0, 0, time.UTC))
	second, err := store.Append(ctx, Message{SessionID: "s1", RunID: "r2", Role: RoleUser, Text: "next"})
	if err != nil {
		t.Fatalf("append second: %v", err)
	}
	if first.ID == "" || second.ID == "" || !second.CreatedAt.After(first.CreatedAt) {
		t.Fatalf("timestamps or ids not assigned: %#v %#v", first, second)
	}

	bySession, err := store.ListBySession(ctx, "s1")
	if err != nil {
		t.Fatalf("list by session: %v", err)
	}
	if len(bySession) != 2 || bySession[0].Text != "hello" || bySession[1].Text != "next" {
		t.Fatalf("session messages = %#v", bySession)
	}
	byRun, err := store.ListByRun(ctx, "s1", "r2")
	if err != nil {
		t.Fatalf("list by run: %v", err)
	}
	if len(byRun) != 1 || byRun[0].Text != "next" {
		t.Fatalf("run messages = %#v", byRun)
	}

	reopened := Open(dataDir)
	reopenedMessages, err := reopened.ListBySession(ctx, "s1")
	if err != nil {
		t.Fatalf("reopened list: %v", err)
	}
	if len(reopenedMessages) != 2 || reopenedMessages[0].ID != first.ID || reopenedMessages[1].ID != second.ID {
		t.Fatalf("reopened messages = %#v", reopenedMessages)
	}
}

func TestProjectionFromEventAndAssistantCoalescing(t *testing.T) {
	ctx := context.Background()
	store := Open(t.TempDir())
	events := []eventbus.Event{
		event("e1", eventbus.KindUserPromptSubmitted, map[string]any{"prompt": "hi"}),
		event("e2", eventbus.KindAssistantDelta, map[string]any{"text": "hel"}),
		event("e3", eventbus.KindAssistantDelta, map[string]any{"text": "lo"}),
		event("e4", eventbus.KindAssistantCompleted, map[string]any{"text": "hello"}),
		event("e5", eventbus.KindToolRequested, map[string]any{"tool": "read_file", "tool_call_id": "tc1"}),
		event("e6", eventbus.KindToolCompleted, map[string]any{
			"tool":         "read_file",
			"tool_call_id": "tc1",
			"input":        map[string]any{"path": "internal/godex/tools/registry.go"},
			"text":         "file contents",
		}),
		event("e7", eventbus.KindToolFailed, map[string]any{"tool": "apply_patch", "tool_call_id": "tc2", "error": "failed"}),
		event("e8", eventbus.KindRunFailed, map[string]any{"error": "run failed"}),
	}
	for _, ev := range events {
		if _, err := store.AppendProjected(ctx, ev); err != nil {
			t.Fatalf("append projected %s: %v", ev.ID, err)
		}
	}

	messages, err := store.ListBySession(ctx, "s1")
	if err != nil {
		t.Fatalf("list: %v", err)
	}
	if len(messages) != 6 {
		t.Fatalf("messages = %#v", messages)
	}
	assertMessage(t, messages[0], RoleUser, "hi", "", "")
	assertMessage(t, messages[1], RoleAssistant, "hello", "", "")
	assertMessage(t, messages[2], RoleTool, "requested", "read_file", "tc1")
	assertMessage(t, messages[3], RoleTool, "read internal/godex/tools/registry.go", "read_file", "tc1")
	assertMessage(t, messages[4], RoleTool, "failed", "apply_patch", "tc2")
	assertMessage(t, messages[5], RoleError, "run failed", "", "")
}

func TestProjectionIgnoresEmptyAndUntrackedEvents(t *testing.T) {
	if got := ProjectionFromEvent(event("e1", eventbus.KindRunStarted, nil)); len(got) != 0 {
		t.Fatalf("run started projection = %#v", got)
	}
	if got := ProjectionFromEvent(event("e2", eventbus.KindUserPromptSubmitted, map[string]any{"prompt": ""})); len(got) != 0 {
		t.Fatalf("empty prompt projection = %#v", got)
	}
}

func event(id string, kind eventbus.Kind, payload any) eventbus.Event {
	return eventbus.Event{
		ID:        id,
		SessionID: "s1",
		RunID:     "r1",
		Kind:      kind,
		Time:      time.Date(2026, 5, 13, 10, 0, 0, 0, time.UTC),
		Payload:   payload,
	}
}

func assertMessage(t *testing.T, msg Message, role string, text string, toolName string, toolCallID string) {
	t.Helper()
	if msg.Role != role || msg.Text != text || msg.ToolName != toolName || msg.ToolCallID != toolCallID {
		t.Fatalf("message = %#v, want role=%s text=%q tool=%q call=%q", msg, role, text, toolName, toolCallID)
	}
}

func fixedClock(now time.Time) func() time.Time {
	return func() time.Time { return now }
}
