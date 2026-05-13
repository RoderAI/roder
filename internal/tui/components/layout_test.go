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

func TestRenderReasoningSummaryAboveComposer(t *testing.T) {
	zones := zone.New()
	t.Cleanup(zones.Close)

	out := zones.Scan(Render(viewmodel.Model{
		Width:            80,
		Height:           24,
		Model:            "gpt-test",
		Provider:         "codex",
		ReasoningSummary: "Checking workspace before editing files.",
		Input:            "> Ask gode to work on this repo",
		InputHeight:      1,
		Status:           "reasoning",
	}, zones))

	if got := lipgloss.Height(out); got != 24 {
		t.Fatalf("height = %d, want 24\n%s", got, out)
	}
	reasoningIndex := strings.Index(out, "REASONING")
	composerIndex := strings.Index(out, "> Ask gode to work on this repo")
	if reasoningIndex < 0 || composerIndex < 0 || reasoningIndex > composerIndex {
		t.Fatalf("reasoning summary should render above composer:\n%s", out)
	}
	for _, want := range []string{"REASONING", "Checking workspace before editing files."} {
		if !strings.Contains(out, want) {
			t.Fatalf("rendered output missing %q:\n%s", want, out)
		}
	}
}

func TestRenderClipsTallToolTranscriptBeforeComposer(t *testing.T) {
	zones := zone.New()
	t.Cleanup(zones.Close)

	messages := make([]viewmodel.Message, 0, 12)
	for i := 0; i < 12; i++ {
		messages = append(messages, viewmodel.Message{
			ID:    "tool-" + string(rune('a'+i)),
			Role:  viewmodel.RoleTool,
			Title: "search_files",
			Body:  strings.Repeat("long tool output line with matches and context ", 16),
		})
	}

	out := zones.Scan(Render(viewmodel.Model{
		Width:       80,
		Height:      18,
		Model:       "gpt-test",
		Provider:    "codex",
		Input:       "> Ask gode to work on this repo",
		InputHeight: 1,
		Messages:    messages,
		Status:      "tool completed: search_files",
	}, zones))

	if got := lipgloss.Height(out); got != 18 {
		t.Fatalf("height = %d, want 18\n%s", got, out)
	}
	if !strings.Contains(out, "> Ask gode to work on this repo") {
		t.Fatalf("composer should stay visible below tall tool output:\n%s", out)
	}
	if !strings.Contains(out, "tool completed: search_files") {
		t.Fatalf("footer should stay visible below tall tool output:\n%s", out)
	}
}

func TestRenderKeepsBottomGutterWhenScrolled(t *testing.T) {
	zones := zone.New()
	t.Cleanup(zones.Close)

	messages := make([]viewmodel.Message, 0, 12)
	for i := 0; i < 12; i++ {
		messages = append(messages, viewmodel.Message{
			ID:   "m" + string(rune('a'+i)),
			Role: viewmodel.RoleTool,
			Body: strings.Repeat("scrolling tool output ", 10),
		})
	}

	out := zones.Scan(Render(viewmodel.Model{
		Width:        80,
		Height:       18,
		Model:        "gpt-test",
		Provider:     "codex",
		Input:        "> Ask gode to work on this repo",
		InputHeight:  1,
		Messages:     messages,
		ScrollOffset: 4,
		Status:       "tool completed: search_files",
	}, zones))

	if got := lipgloss.Height(out); got != 18 {
		t.Fatalf("height = %d, want 18\n%s", got, out)
	}
	lines := strings.Split(out, "\n")
	if len(lines) != 18 {
		t.Fatalf("line count = %d, want 18\n%s", len(lines), out)
	}
	if strings.TrimSpace(lines[len(lines)-1]) != "" {
		t.Fatalf("last line should be reserved bottom gutter, got %q\n%s", lines[len(lines)-1], out)
	}
	if !strings.Contains(lines[len(lines)-2], "scroll 4") {
		t.Fatalf("footer should stay above bottom gutter, got %q\n%s", lines[len(lines)-2], out)
	}
}

func TestErrorConsoleIsBorderlessAndShowsMultilineDetails(t *testing.T) {
	out := ErrorConsole(80, 8, []viewmodel.ErrorLogEntry{{
		Time:   "10:02:20",
		Source: "run",
		Message: strings.Join([]string{
			"OpenAI stream request failed",
			"request: POST https://chatgpt.com/backend-api/codex/responses",
			"status: 400 Bad Request",
			"response_body:",
			`{"detail":"unsupported model"}`,
		}, "\n"),
	}})

	if strings.ContainsAny(out, "┌┐└┘│─") {
		t.Fatalf("error console should not render a border:\n%s", out)
	}
	for _, want := range []string{"ERROR LOG", "request: POST", "status: 400 Bad Request", "unsupported model"} {
		if !strings.Contains(out, want) {
			t.Fatalf("error console missing %q:\n%s", want, out)
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
