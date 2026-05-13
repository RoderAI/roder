package tui

import (
	"context"
	"testing"

	"github.com/pandelisz/gode/internal/godex"
	"github.com/pandelisz/gode/internal/godex/eventbus"
)

func TestModelAppendsAssistantDeltaEvents(t *testing.T) {
	app, err := godex.New(context.Background(), godex.Config{DataDir: t.TempDir(), Workspace: t.TempDir(), Provider: "mock", AutoApprove: true})
	if err != nil {
		t.Fatalf("app: %v", err)
	}
	defer app.Close(context.Background())

	model := New(app)
	updated, _ := model.Update(eventMsg{Event: eventbus.Event{Kind: eventbus.KindAssistantDelta, Payload: map[string]any{"text": "hello"}}})
	got := updated.(Model)
	if len(got.lines) != 1 || got.lines[0] != "assistant: hello" {
		t.Fatalf("lines = %#v", got.lines)
	}
}
