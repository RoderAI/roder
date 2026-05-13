package diffview

import (
	"strings"

	"charm.land/lipgloss/v2"
)

var (
	addedStyle   = lipgloss.NewStyle().Foreground(lipgloss.Color("86"))
	removedStyle = lipgloss.NewStyle().Foreground(lipgloss.Color("203"))
	metaStyle    = lipgloss.NewStyle().Foreground(lipgloss.Color("244"))
)

func IsDiffTool(tool string) bool {
	switch strings.TrimSpace(tool) {
	case "git_diff", "edit", "multi_edit", "write_file":
		return true
	default:
		return false
	}
}

func RenderLines(text string, width int, maxLines int) []string {
	width = max(8, width)
	if maxLines <= 0 {
		maxLines = 24
	}
	rawLines := strings.Split(strings.TrimRight(text, "\n"), "\n")
	if len(rawLines) == 1 && rawLines[0] == "" {
		return []string{""}
	}
	truncated := false
	if len(rawLines) > maxLines {
		rawLines = rawLines[:maxLines]
		truncated = true
	}
	lines := make([]string, 0, len(rawLines)+1)
	for _, raw := range rawLines {
		lines = append(lines, styleLine(trimToWidth(raw, width)))
	}
	if truncated {
		lines = append(lines, metaStyle.Render(trimToWidth("... diff truncated; full result is in the event journal", width)))
	}
	return lines
}

func styleLine(line string) string {
	switch {
	case strings.HasPrefix(line, "+++") || strings.HasPrefix(line, "---") || strings.HasPrefix(line, "@@"):
		return metaStyle.Render(line)
	case strings.HasPrefix(line, "+"):
		return addedStyle.Render(line)
	case strings.HasPrefix(line, "-"):
		return removedStyle.Render(line)
	default:
		return line
	}
}

func trimToWidth(text string, width int) string {
	for lipgloss.Width(text) > width {
		runes := []rune(text)
		if len(runes) == 0 {
			return ""
		}
		text = string(runes[:len(runes)-1])
	}
	return text
}

func max(a, b int) int {
	if a > b {
		return a
	}
	return b
}
