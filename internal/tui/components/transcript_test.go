package components

import (
	"strings"
	"testing"

	"github.com/charmbracelet/x/ansi"
	zone "github.com/lrstanley/bubblezone/v2"
	"github.com/pandelisz/gode/internal/tui/selection"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

func TestTranscriptDetailedReturnsVisibleLineMetadata(t *testing.T) {
	result := TranscriptDetailedWithCache(80, 8, []viewmodel.Message{{
		ID:   "m1",
		Role: viewmodel.RoleUser,
		Body: "hello transcript",
	}}, 0, "", zone.New(), nil, TranscriptOptions{})

	if len(result.Lines) == 0 {
		t.Fatal("expected transcript line metadata")
	}
	found := false
	for _, line := range result.Lines {
		if line.MessageIndex == 0 && line.Text != "" && strings.Contains(line.CopyText, "hello transcript") && !line.Decorative {
			found = true
			break
		}
	}
	if !found {
		t.Fatalf("metadata missing copyable user body row: %#v", result.Lines)
	}
}

func TestTranscriptDetailedHighlightsSelectedRows(t *testing.T) {
	result := TranscriptDetailedWithCache(80, 8, []viewmodel.Message{{
		ID:   "m1",
		Role: viewmodel.RoleAssistant,
		Body: "abcdef",
	}}, 0, "", zone.New(), nil, TranscriptOptions{
		Selection: selection.Range{
			Anchor: selection.Point{Line: 0, Column: 2},
			Focus:  selection.Point{Line: 0, Column: 5},
			Active: true,
		},
	})

	if ansi.Strip(result.View) == result.View {
		t.Fatalf("expected ANSI highlight styling in view:\n%s", result.View)
	}
	if !strings.Contains(ansi.Strip(result.View), "abcdef") {
		t.Fatalf("highlight changed visible transcript:\n%s", result.View)
	}
}

func TestTranscriptDetailedCopyTextDropsWrappedChrome(t *testing.T) {
	result := TranscriptDetailedWithCache(34, 8, []viewmodel.Message{{
		ID:   "m1",
		Role: viewmodel.RoleUser,
		Body: "a markdown [link](https://example.com/docs) wraps cleanly",
	}}, 0, "", zone.New(), nil, TranscriptOptions{})

	var copyRows []selection.TranscriptLineRef
	for _, line := range result.Lines {
		if !line.Decorative {
			copyRows = append(copyRows, line)
		}
	}
	copied := selection.SanitizeTranscriptCopy(copyRows)
	if strings.Contains(copied, "▌") {
		t.Fatalf("copy text kept user rail chrome:\n%s", copied)
	}
	for _, want := range []string{"markdown", "https://example.com/docs"} {
		if !strings.Contains(copied, want) {
			t.Fatalf("copy text missing %q:\n%s", want, copied)
		}
	}
}

func TestTranscriptUserMessageUsesCompactRail(t *testing.T) {
	result := TranscriptDetailedWithCache(50, 8, []viewmodel.Message{{
		ID:   "m1",
		Role: viewmodel.RoleUser,
		Body: "hello user",
	}}, 0, "", zone.New(), nil, TranscriptOptions{})

	if strings.Contains(result.View, "48;5;236") {
		t.Fatalf("did not expect dark gray user message background:\n%s", result.View)
	}
	if !strings.Contains(ansi.Strip(result.View), "▌ hello user") {
		t.Fatalf("expected compact user rail:\n%s", result.View)
	}
	if len(result.Lines) != 1 {
		t.Fatalf("expected only user body row, got %#v", result.Lines)
	}
}

func TestTranscriptAssistantMessageUsesWhiteText(t *testing.T) {
	result := TranscriptDetailedWithCache(50, 8, []viewmodel.Message{{
		ID:   "m1",
		Role: viewmodel.RoleAssistant,
		Body: "final answer",
	}}, 0, "", zone.New(), nil, TranscriptOptions{})

	if !strings.Contains(result.View, "38;5;231") {
		t.Fatalf("expected assistant message body to use white text:\n%s", result.View)
	}
	if strings.Contains(result.View, "38;5;252mfinal answer") {
		t.Fatalf("assistant message body should not use light gray text:\n%s", result.View)
	}
}
