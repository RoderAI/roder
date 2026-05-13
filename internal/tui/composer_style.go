package tui

import (
	"charm.land/bubbles/v2/textarea"
	"charm.land/lipgloss/v2"
)

func applyComposerStyles(input *textarea.Model) {
	styles := input.Styles()
	styles.Focused.CursorLine = lipgloss.NewStyle()
	styles.Focused.Prompt = lipgloss.NewStyle().Foreground(lipgloss.Color("252"))
	styles.Focused.Placeholder = lipgloss.NewStyle().Foreground(lipgloss.Color("244"))
	styles.Focused.Text = lipgloss.NewStyle().Foreground(lipgloss.Color("252"))
	styles.Blurred.CursorLine = lipgloss.NewStyle()
	styles.Blurred.Prompt = lipgloss.NewStyle().Foreground(lipgloss.Color("246"))
	styles.Blurred.Placeholder = lipgloss.NewStyle().Foreground(lipgloss.Color("240"))
	styles.Blurred.Text = lipgloss.NewStyle().Foreground(lipgloss.Color("246"))
	input.SetStyles(styles)
}
