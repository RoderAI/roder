package components

import (
	"strings"
	"testing"

	"github.com/charmbracelet/x/ansi"
	zone "github.com/lrstanley/bubblezone/v2"

	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

func TestTranscriptMarkdownRendering(t *testing.T) {
	msg := viewmodel.Message{
		ID:   "assistant-1",
		Role: "assistant",
		Body: "This is **important**.",
	}

	z := zone.New()
	plain := TranscriptDetailedWithCache(80, 8, []viewmodel.Message{msg}, 0, "", z, nil, TranscriptOptions{MarkdownRendering: false}).View
	rendered := TranscriptDetailedWithCache(80, 8, []viewmodel.Message{msg}, 0, "", z, nil, TranscriptOptions{MarkdownRendering: true}).View

	if !strings.Contains(ansi.Strip(plain), "**important**") {
		t.Fatalf("expected plain transcript to keep markdown markers, got:\n%s", plain)
	}
	strippedRendered := ansi.Strip(rendered)
	if strings.Contains(strippedRendered, "**important**") {
		t.Fatalf("expected markdown transcript to remove markdown markers, got:\n%s", rendered)
	}
	if !strings.Contains(strippedRendered, "important") {
		t.Fatalf("expected markdown transcript to keep rendered text, got:\n%s", rendered)
	}
	if rendered == plain {
		t.Fatal("expected markdown rendering to alter transcript output")
	}
}
