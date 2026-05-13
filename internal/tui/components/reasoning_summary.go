package components

import (
	"strings"

	"charm.land/lipgloss/v2"
)

var (
	reasoningSummaryStyle = lipgloss.NewStyle().
				Foreground(lipgloss.Color("245")).
				Padding(0, 1)
	reasoningSummaryLabelStyle = lipgloss.NewStyle().
					Bold(true).
					Foreground(lipgloss.Color("212"))
)

func ReasoningSummaryHeight(text string, totalHeight int) int {
	if strings.TrimSpace(text) == "" {
		return 0
	}
	if totalHeight < 16 {
		return 2
	}
	return 3
}

func ReasoningSummary(width int, height int, text string) string {
	if height <= 0 {
		return ""
	}
	innerWidth := max(12, width-2)
	lines := []string{reasoningSummaryLabelStyle.Render("REASONING")}
	lines = append(lines, wrapCell(strings.TrimSpace(text), innerWidth)...)
	for len(lines) < height {
		lines = append(lines, "")
	}
	if len(lines) > height {
		lines = lines[len(lines)-height:]
	}
	return reasoningSummaryStyle.Width(width).Height(height).Render(strings.Join(lines, "\n"))
}

func wrapCell(text string, width int) []string {
	if text == "" {
		return []string{""}
	}
	var lines []string
	for _, raw := range strings.Split(text, "\n") {
		words := strings.Fields(raw)
		if len(words) == 0 {
			lines = append(lines, "")
			continue
		}
		line := ""
		for _, word := range words {
			if lipgloss.Width(word) > width {
				if line != "" {
					lines = append(lines, line)
					line = ""
				}
				lines = append(lines, wrapLongCell(word, width)...)
				continue
			}
			if line == "" {
				line = word
				continue
			}
			next := line + " " + word
			if lipgloss.Width(next) > width {
				lines = append(lines, line)
				line = word
				continue
			}
			line = next
		}
		if line != "" {
			lines = append(lines, line)
		}
	}
	if len(lines) == 0 {
		return []string{""}
	}
	return lines
}

func wrapLongCell(word string, width int) []string {
	var lines []string
	line := ""
	for _, r := range word {
		next := line + string(r)
		if line != "" && lipgloss.Width(next) > width {
			lines = append(lines, line)
			line = string(r)
			continue
		}
		line = next
	}
	if line != "" {
		lines = append(lines, line)
	}
	return lines
}
