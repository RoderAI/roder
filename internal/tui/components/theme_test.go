package components

import (
	"strings"
	"testing"

	zone "github.com/lrstanley/bubblezone/v2"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

func TestLightThemeMapsPrimaryTextAwayFromWhite(t *testing.T) {
	withTheme(t, ThemeForDarkBackground(false))

	if got := themeColor(ColorText); got != "16" {
		t.Fatalf("light text color = %q, want black", got)
	}
	if got := themeColor(ColorTextStrong); got != "16" {
		t.Fatalf("light strong text color = %q, want black", got)
	}

	header := Header(80, "codex", "gpt-5.5", "medium", "session", false)
	if strings.Contains(header, "38;5;231") || strings.Contains(header, "38;5;252") {
		t.Fatalf("light header should not render white-ish text:\n%q", header)
	}
	if !strings.Contains(header, "38;5;16") {
		t.Fatalf("light header missing dark text:\n%q", header)
	}
}

func TestLightThemeRendersTranscriptWithDarkBodyText(t *testing.T) {
	withTheme(t, ThemeForDarkBackground(false))

	result := TranscriptDetailedWithCache(60, 8, []viewmodel.Message{{
		ID:   "m1",
		Role: viewmodel.RoleAssistant,
		Body: "final answer",
	}}, 0, "", zone.New(), nil, TranscriptOptions{})

	if strings.Contains(result.View, "38;5;231") || strings.Contains(result.View, "38;5;252") {
		t.Fatalf("light transcript should not render white-ish body text:\n%s", result.View)
	}
	if !strings.Contains(result.View, "38;5;16") {
		t.Fatalf("light transcript missing dark body text:\n%s", result.View)
	}
}

func TestLightThemeRendersToolStatesWithReadableColors(t *testing.T) {
	withTheme(t, ThemeForDarkBackground(false))

	result := TranscriptDetailedWithCache(80, 8, []viewmodel.Message{
		{ID: "tool-1", Role: viewmodel.RoleTool, Title: "shell", Body: "requested\nsleep 8"},
		{ID: "tool-2", Role: viewmodel.RoleTool, Title: "shell", Body: "succeeded\nsleep 8"},
	}, 0, "", zone.New(), nil, TranscriptOptions{})

	if strings.Contains(result.View, "38;5;231") || strings.Contains(result.View, "38;5;252") {
		t.Fatalf("light tool states should not render white-ish text:\n%s", result.View)
	}
	for _, colorCode := range []string{"38;5;25", "38;5;96"} {
		if !strings.Contains(result.View, colorCode) {
			t.Fatalf("light tool state missing color %s:\n%s", colorCode, result.View)
		}
	}
}

func TestThemeChangeInvalidatesTranscriptCache(t *testing.T) {
	cache := NewTranscriptCache()
	messages := []viewmodel.Message{{
		ID:   "m1",
		Role: viewmodel.RoleAssistant,
		Body: "cached answer",
	}}

	withTheme(t, ThemeForDarkBackground(true))
	dark := TranscriptDetailedWithCache(60, 8, messages, 0, "", zone.New(), &cache, TranscriptOptions{})
	if !strings.Contains(dark.View, "38;5;231") {
		t.Fatalf("dark transcript missing expected strong text color:\n%s", dark.View)
	}

	SetTheme(ThemeForDarkBackground(false))
	light := TranscriptDetailedWithCache(60, 8, messages, 0, "", zone.New(), &cache, TranscriptOptions{})
	if !strings.Contains(light.View, "38;5;16") {
		t.Fatalf("theme change should rerender cached transcript with light colors:\n%s", light.View)
	}
}

func withTheme(t *testing.T, theme Theme) {
	t.Helper()
	SetTheme(theme)
	t.Cleanup(func() {
		SetTheme(ThemeForDarkBackground(true))
	})
}
