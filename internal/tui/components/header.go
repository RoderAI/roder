package components

import (
	"strings"

	"charm.land/lipgloss/v2"
)

var (
	headerBarStyle = lipgloss.NewStyle().
			Bold(true).
			Foreground(lipgloss.Color("231")).
			Padding(0, 1)
	headerAccentStyle = lipgloss.NewStyle().
				Bold(true).
				Foreground(lipgloss.Color("212"))
	headerMetaStyle = lipgloss.NewStyle().
			Foreground(lipgloss.Color("252"))
)

func Header(width int, provider string, model string, reasoning string, sessionTitle string, running bool) string {
	left := headerAccentStyle.Render("gode")
	if provider != "" {
		modelLabel := provider + "/" + model
		if reasoning != "" {
			modelLabel += " " + reasoning
		}
		left += headerMetaStyle.Render("  " + modelLabel)
	}
	if strings.TrimSpace(sessionTitle) != "" {
		left += headerMetaStyle.Render("  " + truncateHeader(sessionTitle, 32))
	}

	right := "idle"
	if running {
		right = "running"
	}
	right = headerMetaStyle.Render(right)

	gap := strings.Repeat(" ", max(1, width-lipgloss.Width(left)-lipgloss.Width(right)-2))
	return headerBarStyle.Width(width).Render(left + gap + right)
}

func truncateHeader(text string, width int) string {
	text = strings.TrimSpace(text)
	for lipgloss.Width(text) > width {
		runes := []rune(text)
		if len(runes) == 0 {
			return ""
		}
		text = string(runes[:len(runes)-1])
	}
	return text
}
