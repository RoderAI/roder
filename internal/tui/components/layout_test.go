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
		Input:       "Ask gode to work on this repo",
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
		Input:       "Ask gode to work on this repo",
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

func TestRenderListDialogOverTimeline(t *testing.T) {
	zones := zone.New()
	t.Cleanup(zones.Close)

	out := zones.Scan(Render(viewmodel.Model{
		Width:       80,
		Height:      24,
		Model:       "gpt-test",
		Provider:    "mock",
		Input:       "Ask gode to work on this repo",
		InputHeight: 1,
		Status:      "commands",
		Dialogs: viewmodel.DialogStack{Commands: &viewmodel.ListDialog{
			Kind:  "commands",
			Title: "Commands",
			Help:  "enter insert",
			Items: []viewmodel.ListDialogItem{{
				ID:          "project:test",
				Label:       "/project:test",
				Description: "Run project tests",
				Value:       "project",
				Selected:    true,
			}},
		}},
	}, zones))

	for _, want := range []string{"Commands", "/project:test", "Run project tests", "enter insert"} {
		if !strings.Contains(out, want) {
			t.Fatalf("rendered output missing %q:\n%s", want, out)
		}
	}
}

func TestRenderInlineSlashMenuBelowComposer(t *testing.T) {
	zones := zone.New()
	t.Cleanup(zones.Close)

	out := zones.Scan(Render(viewmodel.Model{
		Width:       80,
		Height:      18,
		Model:       "gpt-test",
		Provider:    "mock",
		Input:       "/",
		InputHeight: 1,
		SlashMenu: &viewmodel.ListDialog{
			Kind: "slash",
			Items: []viewmodel.ListDialogItem{{
				ID:          "goal",
				Label:       "/goal",
				Description: "set, show, pause, resume, clear, or budget the current goal",
				Value:       "builtin",
				Selected:    true,
			}},
		},
		Status: "ready",
	}, zones))

	if got := lipgloss.Height(out); got != 18 {
		t.Fatalf("height = %d, want 18\n%s", got, out)
	}
	inputIndex := strings.Index(out, "/")
	menuIndex := strings.Index(out, "/goal")
	if inputIndex < 0 || menuIndex < 0 || menuIndex < inputIndex {
		t.Fatalf("slash menu should render below composer:\n%s", out)
	}
	for _, want := range []string{"/goal", "set, show, pause"} {
		if !strings.Contains(out, want) {
			t.Fatalf("rendered output missing %q:\n%s", want, out)
		}
	}
}

func TestRenderPermissionDialogOverTimeline(t *testing.T) {
	zones := zone.New()
	t.Cleanup(zones.Close)

	out := zones.Scan(Render(viewmodel.Model{
		Width:       80,
		Height:      24,
		Model:       "gpt-test",
		Provider:    "mock",
		Input:       "Ask gode to work on this repo",
		InputHeight: 1,
		Status:      "permission requested",
		Dialogs: viewmodel.DialogStack{Permissions: &viewmodel.PermissionDialog{
			Title: "Permission",
			Help:  "a allow",
			Requests: []viewmodel.PermissionDialogRequest{{
				ID:       "corr-1",
				Tool:     "write_file",
				Action:   "write",
				Input:    "README.md",
				Selected: true,
			}},
		}},
	}, zones))

	for _, want := range []string{"Permission", "write_file", "README.md", "a allow"} {
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
		Input:        "Ask gode to work on this repo",
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
		Input:            "Ask gode to work on this repo",
		InputHeight:      1,
		Status:           "reasoning",
	}, zones))

	if got := lipgloss.Height(out); got != 24 {
		t.Fatalf("height = %d, want 24\n%s", got, out)
	}
	reasoningIndex := strings.Index(out, "REASONING")
	composerIndex := strings.Index(out, "Ask gode to work on this repo")
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
		Input:       "Ask gode to work on this repo",
		InputHeight: 1,
		Messages:    messages,
		Status:      "tool completed: search_files",
	}, zones))

	if got := lipgloss.Height(out); got != 18 {
		t.Fatalf("height = %d, want 18\n%s", got, out)
	}
	if !strings.Contains(out, "Ask gode to work on this repo") {
		t.Fatalf("composer should stay visible below tall tool output:\n%s", out)
	}
	if !strings.Contains(out, "tool completed: search_files") {
		t.Fatalf("footer should stay visible below tall tool output:\n%s", out)
	}
}

func TestRenderToolCardWithDiffAndMetadata(t *testing.T) {
	zones := zone.New()
	t.Cleanup(zones.Close)

	out := zones.Scan(Render(viewmodel.Model{
		Width:       80,
		Height:      24,
		Model:       "gpt-test",
		Provider:    "codex",
		Input:       "Ask gode to work on this repo",
		InputHeight: 1,
		Messages: []viewmodel.Message{{
			ID:    "tool-diff",
			Role:  viewmodel.RoleTool,
			Title: "git_diff",
			Body: strings.Join([]string{
				"diff --git a/main.go b/main.go",
				"@@ -1 +1 @@",
				"-old",
				"+new",
				"hook: allow",
			}, "\n"),
		}},
		Status: "tool completed: git_diff",
	}, zones))

	for _, want := range []string{"TOOL", "git_diff", "diff --git", "-old", "+new"} {
		if !strings.Contains(out, want) {
			t.Fatalf("rendered output missing %q:\n%s", want, out)
		}
	}
	for i, line := range strings.Split(out, "\n") {
		if lipgloss.Width(line) > 80 {
			t.Fatalf("line %d width %d exceeds viewport:\n%s", i, lipgloss.Width(line), out)
		}
	}
}

func TestRenderToolCardShowsHookAndPermissionMetadata(t *testing.T) {
	zones := zone.New()
	t.Cleanup(zones.Close)

	out := zones.Scan(Render(viewmodel.Model{
		Width:       80,
		Height:      24,
		Model:       "gpt-test",
		Provider:    "codex",
		Input:       "Ask gode to work on this repo",
		InputHeight: 1,
		Messages: []viewmodel.Message{
			{ID: "tool-shell", Role: viewmodel.RoleTool, Title: "shell", Body: "ok\nhook: allow"},
			{ID: "tool-perm", Role: viewmodel.RoleTool, Title: "write_file", Body: "permission requested\naction: write\npath: README.md"},
		},
		Status: "permission requested",
	}, zones))

	for _, want := range []string{"TOOL", "shell", "HOOK:", "allow", "write_file", "ACTION:", "README.md"} {
		if !strings.Contains(out, want) {
			t.Fatalf("rendered output missing %q:\n%s", want, out)
		}
	}
}

func TestRenderFooterUsesBottomRowWhenScrolled(t *testing.T) {
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
		Input:        "Ask gode to work on this repo",
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
	if !strings.Contains(lines[len(lines)-1], "scroll 4") {
		t.Fatalf("footer should render on bottom row, got %q\n%s", lines[len(lines)-1], out)
	}
}

func TestFooterShowsContextLeftWithErrorsAndScroll(t *testing.T) {
	out := Footer(80, 6, "ready", true, 2, "ctx 88%")

	for _, want := range []string{"ready", "errors open 2", "ctx 88%", "scroll 6"} {
		if !strings.Contains(out, want) {
			t.Fatalf("footer missing %q:\n%s", want, out)
		}
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
		{ID: "m3", Role: viewmodel.RoleSystem, Body: "three"},
	}

	got := visibleMessages(messages, 80, 3, 1, nil)
	if len(got) != 2 {
		t.Fatalf("visible message count = %d, want 2: %#v", len(got), got)
	}
	if got[0].id != "m2" || got[1].id != "m3" {
		t.Fatalf("visible ids = %#v, want m2,m3", got)
	}
}
