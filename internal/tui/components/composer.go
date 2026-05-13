package components

import "charm.land/lipgloss/v2"

func Composer(width int, input string) string {
	return lipgloss.NewStyle().
		Width(width).
		Border(lipgloss.NormalBorder(), true).
		BorderForeground(lipgloss.Color("240")).
		Render(input)
}
