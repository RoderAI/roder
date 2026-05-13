package components

import (
	"strings"
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

func TestRenderSettingsDialogOverTimeline(t *testing.T) {
	zones := zone.New()
	t.Cleanup(zones.Close)

	out := zones.Scan(Render(viewmodel.Model{
		Width:       80,
		Height:      24,
		Model:       "gpt-test",
		Provider:    "mock",
		Input:       "> Ask gode to work on this repo",
		InputHeight: 1,
		Messages: []viewmodel.Message{{
			ID:   "m1",
			Role: viewmodel.RoleUser,
			Body: "timeline stays visible",
		}},
		Status: "settings",
		Settings: &viewmodel.SettingsDialog{
			Title:  "Settings",
			Screen: viewmodel.SettingsScreenMenu,
			MenuItems: []viewmodel.SettingsMenuItem{{
				ID:          "models",
				Label:       "Models",
				Description: "Choose default model",
				Value:       "gpt-next",
				Selected:    true,
			}},
		},
	}, zones))

	if got := lipgloss.Height(out); got != 24 {
		t.Fatalf("height = %d, want 24\n%s", got, out)
	}
	for _, want := range []string{"Settings", "Models", "gpt-next", "gode", "timeline stays visible"} {
		if !strings.Contains(out, want) {
			t.Fatalf("rendered output missing %q:\n%s", want, out)
		}
	}
}

func TestRenderErrorLogBelowComposer(t *testing.T) {
	zones := zone.New()
	t.Cleanup(zones.Close)

	out := zones.Scan(Render(viewmodel.Model{
		Width:        80,
		Height:       24,
		Model:        "gpt-test",
		Provider:     "codex",
		Input:        "> Ask gode to work on this repo",
		InputHeight:  1,
		ShowErrorLog: true,
		ErrorLog: []viewmodel.ErrorLogEntry{{
			Time:    "10:02:20",
			Source:  "run",
			Message: `POST "https://chatgpt.com/backend-api/codex/responses": 400 Bad Request`,
		}},
		Status: "run failed - ctrl+l errors",
	}, zones))

	if got := lipgloss.Height(out); got != 24 {
		t.Fatalf("height = %d, want 24\n%s", got, out)
	}
	for _, want := range []string{"ERROR LOG", "ctrl+l close", "400 Bad Request"} {
		if !strings.Contains(out, want) {
			t.Fatalf("rendered output missing %q:\n%s", want, out)
		}
	}
}

func TestVisibleMessagesUsesLineScroll(t *testing.T) {
	messages := []viewmodel.Message{
		{ID: "m1", Role: viewmodel.RoleUser, Body: "one"},
		{ID: "m2", Role: viewmodel.RoleAssistant, Body: "two"},
		{ID: "m3", Role: viewmodel.RoleTool, Body: "three"},
	}

	got := visibleMessages(messages, 80, 3, 1, nil)
	if len(got) != 2 {
		t.Fatalf("visible message count = %d, want 2: %#v", len(got), got)
	}
	if got[0].id != "m2" || got[1].id != "m3" {
		t.Fatalf("visible ids = %#v, want m2,m3", got)
	}
}
