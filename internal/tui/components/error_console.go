package components

import (
	"strings"

	"charm.land/lipgloss/v2"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

var (
	errorConsoleStyle = lipgloss.NewStyle().
				Padding(0, 1)
	errorConsoleTitleStyle = lipgloss.NewStyle().
				Bold(true).
				Foreground(lipgloss.Color("231")).
				Background(lipgloss.Color("160")).
				Padding(0, 1)
	errorConsoleMetaStyle = lipgloss.NewStyle().
				Foreground(lipgloss.Color("244"))
	errorConsoleTextStyle = lipgloss.NewStyle().
				Foreground(lipgloss.Color("252"))
)

func ErrorConsole(width int, height int, entries []viewmodel.ErrorLogEntry) string {
	contentWidth := max(20, width-2)
	contentHeight := max(1, height)
	lines := []string{errorConsoleTitleStyle.Render("ERROR LOG") + " " + errorConsoleMetaStyle.Render("ctrl+l close")}
	if len(entries) == 0 {
		lines = append(lines, errorConsoleTextStyle.Render("No errors recorded."))
	} else {
		lines = append(lines, errorConsoleLines(entries, contentWidth, max(0, contentHeight-1))...)
	}
	lines = fitLines(lines, contentWidth, contentHeight)
	return errorConsoleStyle.Width(max(20, width-2)).Height(max(1, height)).Render(strings.Join(lines, "\n"))
}

func errorConsoleLines(entries []viewmodel.ErrorLogEntry, width int, maxLines int) []string {
	if maxLines <= 0 {
		return nil
	}
	lines := make([]string, 0, maxLines)
	for i := len(entries) - 1; i >= 0 && len(lines) < maxLines; i-- {
		entry := entries[i]
		source := entry.Source
		if source == "" {
			source = "error"
		}
		meta := strings.TrimSpace(entry.Time + " " + source)
		if meta != "" {
			lines = append(lines, errorConsoleMetaStyle.Render(truncateCell(meta, width)))
		}
		for _, line := range wrapText(entry.Message, width) {
			if len(lines) >= maxLines {
				break
			}
			lines = append(lines, errorConsoleTextStyle.Render(line))
		}
	}
	return lines
}

func fitLines(lines []string, width int, height int) []string {
	if len(lines) > height {
		lines = lines[:height]
	}
	for len(lines) < height {
		lines = append(lines, "")
	}
	for i := range lines {
		lines[i] = padLine(lines[i], width)
	}
	return lines
}
