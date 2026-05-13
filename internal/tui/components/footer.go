package components

import (
	"fmt"
	"strings"

	"charm.land/lipgloss/v2"
)

var footerStyle = lipgloss.NewStyle().
	Foreground(lipgloss.Color("248")).
	Padding(0, 1)

func Footer(width int, scrollOffset int, status string, showErrorLog bool, errorCount int) string {
	left := "enter send  ctrl+p settings  ctrl+l errors  pgup/pgdn scroll  end follow  esc quit"
	if width < 72 {
		left = "enter send  ctrl+p settings  ctrl+l errors  wheel scroll  esc quit"
	}
	if status != "" {
		left = status
	}
	logState := "errors"
	if showErrorLog {
		logState = "errors open"
	}
	right := fmt.Sprintf("%s %d  scroll %d", logState, errorCount, scrollOffset)
	if lipgloss.Width(left)+lipgloss.Width(right)+2 > width {
		left = truncateCell(left, max(8, width-lipgloss.Width(right)-3))
	}
	gap := strings.Repeat(" ", max(1, width-lipgloss.Width(left)-lipgloss.Width(right)-2))
	return footerStyle.Width(width).Render(left + gap + right)
}

func truncateCell(text string, width int) string {
	if lipgloss.Width(text) <= width {
		return text
	}
	var out string
	for _, r := range text {
		if lipgloss.Width(out+string(r)+"...") > width {
			return out + "..."
		}
		out += string(r)
	}
	return out
}
