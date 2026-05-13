package components

import (
	"strings"

	"charm.land/lipgloss/v2"
)

func Transcript(width int, height int, lines []string) string {
	return lipgloss.NewStyle().
		Width(width).
		Height(height).
		Render(strings.Join(tail(lines, height), "\n"))
}

func tail(lines []string, limit int) []string {
	if len(lines) <= limit {
		return lines
	}
	return lines[len(lines)-limit:]
}
