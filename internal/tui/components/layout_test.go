package components

import (
	"testing"

	"charm.land/lipgloss/v2"
	zone "github.com/lrstanley/bubblezone/v2"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

func TestRenderFillsRequestedHeight(t *testing.T) {
	zones := zone.New()
	t.Cleanup(zones.Close)

	out := zones.Scan(Render(viewmodel.Model{
		Width:       80,
		Height:      24,
		Model:       "mock",
		Provider:    "mock",
		Input:       "> Ask gode to work on this repo",
		InputHeight: 1,
		Messages: []viewmodel.Message{{
			ID:   "m1",
			Role: viewmodel.RoleUser,
			Body: "hello",
		}},
		Status: "ready",
	}, zones))

	if got := lipgloss.Height(out); got != 24 {
		t.Fatalf("height = %d, want 24\n%s", got, out)
	}
}

func TestVisibleMessagesUsesLineScroll(t *testing.T) {
	messages := []viewmodel.Message{
		{ID: "m1", Role: viewmodel.RoleUser, Body: "one"},
		{ID: "m2", Role: viewmodel.RoleAssistant, Body: "two"},
		{ID: "m3", Role: viewmodel.RoleTool, Body: "three"},
	}

	got := visibleMessages(messages, 80, 3, 1)
	if len(got) != 2 {
		t.Fatalf("visible message count = %d, want 2: %#v", len(got), got)
	}
	if got[0].id != "m2" || got[1].id != "m3" {
		t.Fatalf("visible ids = %#v, want m2,m3", got)
	}
}
