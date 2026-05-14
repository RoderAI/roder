package components

import (
	"strings"
	"testing"

	"github.com/charmbracelet/x/ansi"
	zone "github.com/lrstanley/bubblezone/v2"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

func TestTranscriptMinimalSummarizesToolCalls(t *testing.T) {
	result := TranscriptDetailedWithCache(100, 10, []viewmodel.Message{{
		ID:    "tool-grep",
		Role:  viewmodel.RoleTool,
		Title: "grep",
		Body:  "/Users/pz/w/gode/internal/godex/eventbus/bus_test.go:17: first := Event{Kind: KindRunStarted}\n/Users/pz/w/gode/internal/godex/eventbus/bus_test.go:32: if gotFirst.Kind != KindRunStarted",
	}}, 0, "", zone.New(), nil, TranscriptOptions{
		TimelineStyle: viewmodel.TimelineStyleMinimal,
	})

	visible := ansi.Strip(result.View)
	if !strings.Contains(visible, "grep") || !strings.Contains(visible, "bus_test.go:17") {
		t.Fatalf("minimal timeline missing tool summary:\n%s", visible)
	}
	if strings.Contains(visible, "(") || strings.Contains(visible, ")") {
		t.Fatalf("minimal timeline should not wrap tool summaries in brackets:\n%s", visible)
	}
	if strings.Contains(visible, "bus_test.go:32") {
		t.Fatalf("minimal timeline should hide detailed tool output:\n%s", visible)
	}
}

func TestTranscriptMinimalToolRowsAreContiguousAndStatusColored(t *testing.T) {
	result := TranscriptDetailedWithCache(100, 10, []viewmodel.Message{
		{ID: "tool-1", Role: viewmodel.RoleTool, Title: "read_file", Body: "requested"},
		{ID: "tool-2", Role: viewmodel.RoleTool, Title: "read_file", Body: "read src/App.tsx"},
		{ID: "tool-3", Role: viewmodel.RoleTool, Title: "apply_patch", Body: "failed: status 128"},
	}, 0, "", zone.New(), nil, TranscriptOptions{
		TimelineStyle: viewmodel.TimelineStyleMinimal,
	})

	visible := ansi.Strip(result.View)
	if strings.Contains(visible, "\n\n") {
		t.Fatalf("tool rows should not have blank gaps:\n%s", visible)
	}
	if strings.Contains(visible, "requested") || strings.Contains(visible, "read src/App.tsx") || strings.Contains(visible, "running") {
		t.Fatalf("tool summaries should omit requested/read filler:\n%s", visible)
	}
	for _, want := range []string{"read_file", "src/App.tsx", "apply_patch", "failed: status 128"} {
		if !strings.Contains(visible, want) {
			t.Fatalf("tool timeline missing %q:\n%s", want, visible)
		}
	}
	for _, colorCode := range []string{"38;5;75", "38;5;183", "38;5;196"} {
		if !strings.Contains(result.View, colorCode) {
			t.Fatalf("tool timeline missing status color %s:\n%s", colorCode, result.View)
		}
	}
	if !strings.Contains(visible, "○ read_file") || !strings.Contains(visible, "● apply_patch") {
		t.Fatalf("tool timeline missing running/succeeded markers:\n%s", visible)
	}
}

func TestTranscriptDefaultsToMinimalTimeline(t *testing.T) {
	result := TranscriptDetailedWithCache(100, 10, []viewmodel.Message{{
		ID:    "tool-grep",
		Role:  viewmodel.RoleTool,
		Title: "grep",
		Body:  "first match\nsecond match",
	}}, 0, "", zone.New(), nil, TranscriptOptions{})

	visible := ansi.Strip(result.View)
	if !strings.Contains(visible, "grep") || !strings.Contains(visible, "first match") {
		t.Fatalf("default timeline missing tool summary:\n%s", visible)
	}
	if strings.Contains(visible, "second match") {
		t.Fatalf("default timeline should hide detailed tool output:\n%s", visible)
	}
}

func TestTranscriptDetailedKeepsToolOutput(t *testing.T) {
	result := TranscriptDetailedWithCache(100, 10, []viewmodel.Message{{
		ID:    "tool-grep",
		Role:  viewmodel.RoleTool,
		Title: "grep",
		Body:  "first match\nsecond match",
	}}, 0, "", zone.New(), nil, TranscriptOptions{
		TimelineStyle: viewmodel.TimelineStyleDetailed,
	})

	visible := ansi.Strip(result.View)
	if !strings.Contains(visible, "first match") || !strings.Contains(visible, "second match") {
		t.Fatalf("detailed timeline should render full tool output:\n%s", visible)
	}
}
