package tui

import (
	"context"
	"strings"
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
	if len(got.messages) != 1 || got.messages[0].Body != "hello" {
		t.Fatalf("messages = %#v", got.messages)
	}
}

func TestModelCoalescesAssistantDeltaEvents(t *testing.T) {
	model := New(nil)
	updated, _ := model.Update(eventMsg{Event: eventbus.Event{Kind: eventbus.KindAssistantDelta, Payload: map[string]any{"text": "hel"}}})
	updated, _ = updated.Update(eventMsg{Event: eventbus.Event{Kind: eventbus.KindAssistantDelta, Payload: map[string]any{"text": "lo"}}})

	got := updated.(Model)
	if len(got.messages) != 1 || got.messages[0].Body != "hello" {
		t.Fatalf("messages = %#v", got.messages)
	}
}

func TestModelScrollState(t *testing.T) {
	model := New(nil)
	model.width = 40
	model.height = 10
	for i := 0; i < 10; i++ {
		model.addMessage("user", "", "message")
	}

	model.scrollBy(3)
	if model.scrollOffset != 3 || model.followTail {
		t.Fatalf("scrollOffset=%d followTail=%v", model.scrollOffset, model.followTail)
	}

	model.follow()
	if model.scrollOffset != 0 || !model.followTail {
		t.Fatalf("scrollOffset=%d followTail=%v", model.scrollOffset, model.followTail)
	}
}

func TestNewModelFocusesComposer(t *testing.T) {
	model := New(nil)
	if !model.input.Focused() {
		t.Fatal("composer should be focused")
	}
}

func TestComposerDoesNotPaintCursorLineBackground(t *testing.T) {
	model := New(nil)
	view := model.input.View()
	if strings.Contains(view, "\x1b[40m") || strings.Contains(view, "\x1b[48;5;0m") {
		t.Fatalf("composer view contains black background ANSI: %q", view)
	}
}
