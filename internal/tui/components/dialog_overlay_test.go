package components

import (
	"strings"
	"testing"

	"charm.land/lipgloss/v2"
)

func TestOverlayDialogPreservesTimelineOutsideDialogBounds(t *testing.T) {
	base := strings.Join([]string{
		"header",
		"left timeline text                         right timeline text",
		"footer",
	}, "\n")
	box := lipgloss.NewStyle().Width(12).Render("Settings")

	out := overlayDialogBox(base, 64, 3, box)
	lines := strings.Split(out, "\n")
	if len(lines) != 3 {
		t.Fatalf("line count = %d, want 3\n%s", len(lines), out)
	}
	if !strings.Contains(lines[1], "left timeline text") {
		t.Fatalf("overlay should preserve left-side timeline content:\n%s", out)
	}
	if !strings.Contains(lines[1], "right timeline text") {
		t.Fatalf("overlay should preserve right-side timeline content:\n%s", out)
	}
	if !strings.Contains(lines[1], "Settings") {
		t.Fatalf("overlay should render dialog content:\n%s", out)
	}
}
