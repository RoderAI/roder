package components

import "charm.land/lipgloss/v2"

func Header(width int, provider string, model string) string {
	title := "gode"
	if provider != "" {
		title += "  " + provider + "/" + model
	}
	return lipgloss.NewStyle().
		Bold(true).
		Foreground(lipgloss.Color("212")).
		Width(width).
		Render(title)
}
