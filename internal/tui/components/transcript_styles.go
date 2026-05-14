package components

import "charm.land/lipgloss/v2"

var (
	transcriptStyle    lipgloss.Style
	emptyStyle         lipgloss.Style
	messageHoverStyle  lipgloss.Style
	bodyStyle          lipgloss.Style
	assistantBodyStyle lipgloss.Style
	userMessageStyle   lipgloss.Style
	userRailStyle      lipgloss.Style
	metaPrefixStyle    lipgloss.Style
	metaTitleStyle     lipgloss.Style
	errorPrefixStyle   lipgloss.Style
	toolTitleStyle     lipgloss.Style
	toolMetaStyle      lipgloss.Style
)

func resetTranscriptStyles() {
	transcriptStyle = lipgloss.NewStyle().
		Padding(1, 1)
	emptyStyle = lipgloss.NewStyle().
		Foreground(ThemeColor(ColorMuted)).
		Italic(true)
	messageHoverStyle = lipgloss.NewStyle().
		Foreground(ThemeColor(ColorTextStrong))
	bodyStyle = lipgloss.NewStyle().
		Foreground(ThemeColor(ColorText))
	assistantBodyStyle = lipgloss.NewStyle().
		Foreground(ThemeColor(ColorTextStrong))
	userMessageStyle = lipgloss.NewStyle().
		Foreground(ThemeColor(ColorText))
	userRailStyle = lipgloss.NewStyle().
		Foreground(ThemeColor(ColorAccent))
	metaPrefixStyle = lipgloss.NewStyle().
		Foreground(ThemeColor(ColorSubtle))
	metaTitleStyle = lipgloss.NewStyle().
		Foreground(ThemeColor(ColorAccentSoft)).
		Bold(true)
	errorPrefixStyle = lipgloss.NewStyle().
		Foreground(ThemeColor(ColorError)).
		Bold(true)
	toolTitleStyle = lipgloss.NewStyle().
		Foreground(ThemeColor(ColorTool)).
		Bold(true)
	toolMetaStyle = lipgloss.NewStyle().
		Foreground(ThemeColor(ColorMuted))
}
