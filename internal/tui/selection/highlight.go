package selection

import (
	"strings"

	"charm.land/lipgloss/v2"
	"github.com/charmbracelet/x/ansi"
)

func HighlightLine(line string, startCol int, endCol int, style lipgloss.Style) string {
	visibleWidth := len([]rune(ansi.Strip(line)))
	startCol = clampInt(startCol, 0, visibleWidth)
	endCol = clampInt(endCol, 0, visibleWidth)
	if startCol >= endCol {
		return line
	}

	var before, selected, after strings.Builder
	visibleCol := 0
	for i := 0; i < len(line); {
		if sequence, width := ansiSequence(line[i:]); width > 0 {
			target := &before
			switch {
			case visibleCol >= endCol:
				target = &after
			case visibleCol >= startCol:
				target = &selected
			}
			target.WriteString(sequence)
			i += width
			continue
		}

		char, width := nextRune(line[i:])
		target := &before
		switch {
		case visibleCol >= endCol:
			target = &after
		case visibleCol >= startCol:
			target = &selected
		}
		target.WriteString(char)
		visibleCol++
		i += width
	}

	if selected.Len() == 0 {
		return line
	}
	return before.String() + style.Render(selected.String()) + after.String()
}

func nextRune(text string) (string, int) {
	for i := range text {
		if i > 0 {
			return text[:i], i
		}
	}
	return text, len(text)
}

func ansiSequence(text string) (string, int) {
	if len(text) < 2 || text[0] != '\x1b' {
		return "", 0
	}
	switch text[1] {
	case '[':
		for i := 2; i < len(text); i++ {
			if text[i] >= 0x40 && text[i] <= 0x7e {
				return text[:i+1], i + 1
			}
		}
	case ']':
		for i := 2; i < len(text); i++ {
			if text[i] == '\a' {
				return text[:i+1], i + 1
			}
			if i+1 < len(text) && text[i] == '\x1b' && text[i+1] == '\\' {
				return text[:i+2], i + 2
			}
		}
	default:
		return text[:2], 2
	}
	return "", 0
}
