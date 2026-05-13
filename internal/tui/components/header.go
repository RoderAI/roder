package components

import (
	"strings"

	"charm.land/lipgloss/v2"
)

var (
	headerBarStyle = lipgloss.NewStyle().
			Bold(true).
			Foreground(lipgloss.Color("231")).
			Background(lipgloss.Color("235")).
			Padding(0, 1)
	headerAccentStyle = lipgloss.NewStyle().
				Bold(true).
				Foreground(lipgloss.Color("212"))
	headerMetaStyle = lipgloss.NewStyle().
			Foreground(lipgloss.Color("252")).
			Background(lipgloss.Color("235"))
)

func Header(width int, provider string, model string, running bool) string {
	left := headerAccentStyle.Render("gode")
	if provider != "" {
		left += headerMetaStyle.Render("  " + provider + "/" + model)
	}

	right := "idle"
	if running {
		right = "running"
	}
	right = headerMetaStyle.Render(right)

	gap := strings.Repeat(" ", max(1, width-lipgloss.Width(left)-lipgloss.Width(right)-2))
	return headerBarStyle.Width(width).Render(left + gap + right)
}
