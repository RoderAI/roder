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
	if strings.Contains(visible, "bus_test.go:32") {
		t.Fatalf("minimal timeline should hide detailed tool output:\n%s", visible)
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
