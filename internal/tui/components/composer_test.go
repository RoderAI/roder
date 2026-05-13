package components

import (
	"strings"
	"testing"

	"github.com/charmbracelet/x/ansi"
	zone "github.com/lrstanley/bubblezone/v2"
	"github.com/pandelisz/gode/internal/tui/selection"
)

func TestComposerSelectionHighlightsWithoutChangingValue(t *testing.T) {
	value := "select this text"
	out := ComposerWithSelection(60, "rendered textarea", ComposerOptions{
		Value: value,
		Selection: selection.OffsetRange{
			Anchor: 7,
			Focus:  11,
			Active: true,
		},
	}, zone.New())

	visible := ansi.Strip(out)
	if !strings.Contains(visible, value) {
		t.Fatalf("visible composer = %q, want raw value %q", visible, value)
	}
	if strings.Contains(visible, "rendered textarea") {
		t.Fatalf("selected composer should render raw value, not placeholder/cursor text:\n%s", visible)
	}
	if !strings.Contains(out, "this") || out == value {
		t.Fatalf("selection was not highlighted:\n%s", out)
	}
	if value != "select this text" {
		t.Fatalf("composer value mutated to %q", value)
	}
}

func TestComposerSelectionDoesNotCopyPlaceholder(t *testing.T) {
	got := ComposerSelectionText("", selection.OffsetRange{Anchor: 0, Focus: 10, Active: true})
	if got != "" {
		t.Fatalf("empty composer selection copied placeholder-like text: %q", got)
	}
}
