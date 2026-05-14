package components

import (
	"reflect"
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

func TestTranscriptInsertsConsistentMessageGaps(t *testing.T) {
	got := visibleMessages([]viewmodel.Message{
		{ID: "m1", Role: viewmodel.RoleUser, Body: "hi"},
		{ID: "m2", Role: viewmodel.RoleAssistant, Body: "hello"},
		{ID: "m3", Role: viewmodel.RoleUser, Body: "again"},
	}, 80, 10, 0, nil)

	var lines []string
	for _, item := range got {
		for _, line := range item.lines {
			lines = append(lines, ansi.Strip(line.text))
		}
	}
	want := []string{"▌ hi", "", "hello", "", "▌ again"}
	if !reflect.DeepEqual(lines, want) {
		t.Fatalf("transcript lines = %#v, want %#v", lines, want)
	}
	if !got[1].lines[0].ref.Decorative || got[1].lines[0].ref.LogicalLine != -1 {
		t.Fatalf("message gap should be decorative metadata: %#v", got[1].lines[0].ref)
	}
}

func TestTranscriptTrimsBodyEdgeBlanksBeforeMessageGaps(t *testing.T) {
	got := visibleMessagesWithOptions([]viewmodel.Message{
		{ID: "m1", Role: viewmodel.RoleUser, Body: "hi"},
		{ID: "m2", Role: viewmodel.RoleAssistant, Body: "\n\nHi! What would you like help with?\n\n"},
		{ID: "m3", Role: viewmodel.RoleUser, Body: "what are you capable of"},
	}, 80, 10, 0, nil, viewmodel.TimelineStyleMinimal, true)

	var lines []string
	for _, item := range got {
		for _, line := range item.lines {
			lines = append(lines, strings.TrimSpace(ansi.Strip(line.text)))
		}
	}
	want := []string{"▌ hi", "", "Hi! What would you like help with?", "", "▌ what are you capable of"}
	if !reflect.DeepEqual(lines, want) {
		t.Fatalf("transcript lines = %#v, want %#v", lines, want)
	}
}

func TestTranscriptAssistantPhaseMessagesStartFlushLeft(t *testing.T) {
	result := TranscriptDetailedWithCache(50, 8, []viewmodel.Message{{
		ID:    "m1",
		Role:  viewmodel.RoleAssistant,
		Title: "commentary",
		Body:  "phase message",
	}}, 0, "", zone.New(), nil, TranscriptOptions{})

	for _, line := range strings.Split(ansi.Strip(result.View), "\n") {
		if strings.Contains(line, "phase message") {
			if strings.HasPrefix(line, " ") {
				t.Fatalf("assistant phase message should start flush left, got %q\n%s", line, result.View)
			}
			return
		}
	}
	t.Fatalf("assistant phase message missing:\n%s", result.View)
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
