package components

import "charm.land/lipgloss/v2"

var composerStyle = lipgloss.NewStyle().
	Border(lipgloss.NormalBorder(), true).
	BorderForeground(lipgloss.Color("212")).
	Padding(0, 1)

func Composer(width int, input string) string {
	return composerStyle.
		Width(max(20, width-2)).
		Render(input)
}
