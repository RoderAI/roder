package components

import "charm.land/lipgloss/v2"

func Footer(width int) string {
	return lipgloss.NewStyle().
		Faint(true).
		Width(width).
		Render("enter send  esc quit")
}
