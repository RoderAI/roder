package selection

import (
	"strings"
	"testing"

	"charm.land/lipgloss/v2"
	"github.com/charmbracelet/x/ansi"
)

func TestHighlightLinePreservesANSIStyling(t *testing.T) {
	line := "\x1b[31mred\x1b[0m plain \x1b[34mblue\x1b[0m"
	style := lipgloss.NewStyle().Background(lipgloss.Color("212"))

	got := HighlightLine(line, 1, 13, style)

	if ansi.Strip(got) != ansi.Strip(line) {
		t.Fatalf("visible text changed:\n got: %q\nwant: %q", ansi.Strip(got), ansi.Strip(line))
	}
	for _, sequence := range []string{"\x1b[31m", "\x1b[34m", "\x1b[0m"} {
		if !strings.Contains(got, sequence) {
			t.Fatalf("highlighted line lost ANSI sequence %q:\n%q", sequence, got)
		}
	}
	if got == line {
		t.Fatalf("highlight did not add styling:\n%q", got)
	}
}

func TestHighlightLineMiddleSpan(t *testing.T) {
	style := lipgloss.NewStyle().Foreground(lipgloss.Color("15")).Background(lipgloss.Color("57"))

	got := HighlightLine("abcdef", 2, 5, style)

	if ansi.Strip(got) != "abcdef" {
		t.Fatalf("visible text = %q", ansi.Strip(got))
	}
	if !strings.Contains(got, "cde") {
		t.Fatalf("highlighted middle span missing selected text:\n%q", got)
	}
	if got == "abcdef" {
		t.Fatal("expected highlighted output to differ from input")
	}
}
