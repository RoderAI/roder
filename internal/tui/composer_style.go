package tui

import (
	"charm.land/bubbles/v2/textarea"
	"charm.land/lipgloss/v2"
	"github.com/pandelisz/gode/internal/tui/components"
)

func applyComposerStyles(input *textarea.Model) {
	styles := input.Styles()
	styles.Focused.CursorLine = lipgloss.NewStyle()
	styles.Focused.Prompt = lipgloss.NewStyle().Foreground(components.ThemeColor(components.ColorText))
	styles.Focused.Placeholder = lipgloss.NewStyle().Foreground(components.ThemeColor(components.ColorMuted))
	styles.Focused.Text = lipgloss.NewStyle().Foreground(components.ThemeColor(components.ColorText))
	styles.Blurred.CursorLine = lipgloss.NewStyle()
	styles.Blurred.Prompt = lipgloss.NewStyle().Foreground(components.ThemeColor(components.ColorSubtle))
	styles.Blurred.Placeholder = lipgloss.NewStyle().Foreground(components.ThemeColor(components.ColorMuted))
	styles.Blurred.Text = lipgloss.NewStyle().Foreground(components.ThemeColor(components.ColorSubtle))
	input.SetStyles(styles)
}
