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

func TestComposerBorderUsesNormalModeColorByDefault(t *testing.T) {
	zones := zone.New()
	t.Cleanup(zones.Close)

	out := ComposerWithSelection(60, "hello", ComposerOptions{}, zones)

	assertANSIColor(t, out, "38;5;244")
	assertNoANSIColor(t, out, "38;5;212")
}

func TestComposerBorderUsesAutoApproveColor(t *testing.T) {
	zones := zone.New()
	t.Cleanup(zones.Close)

	out := ComposerWithSelection(60, "hello", ComposerOptions{AutoApprove: true}, zones)

	assertANSIColor(t, out, "38;5;212")
	assertNoANSIColor(t, out, "38;5;244")
}

func TestComposerSelectionDoesNotCopyPlaceholder(t *testing.T) {
	got := ComposerSelectionText("", selection.OffsetRange{Anchor: 0, Focus: 10, Active: true})
	if got != "" {
		t.Fatalf("empty composer selection copied placeholder-like text: %q", got)
	}
}

func assertANSIColor(t *testing.T, out string, color string) {
	t.Helper()
	if !strings.Contains(out, color) {
		t.Fatalf("rendered output missing ANSI color %s:\n%q", color, out)
	}
}

func assertNoANSIColor(t *testing.T, out string, color string) {
	t.Helper()
	if strings.Contains(out, color) {
		t.Fatalf("rendered output should not include ANSI color %s:\n%q", color, out)
	}
}
