package components

import (
	"strings"

	"charm.land/glamour/v2"
	"github.com/charmbracelet/x/ansi"
)

func markdownBodyLines(text string, width int) []renderedLine {
	rendered, err := renderMarkdown(text, width)
	if err != nil || strings.TrimSpace(rendered) == "" {
		return wrappedBodyLines(text, width)
	}
	rawLines := strings.Split(strings.TrimRight(rendered, "\n"), "\n")
	lines := make([]renderedLine, 0, len(rawLines))
	for _, line := range rawLines {
		lines = append(lines, bodyRenderedLine(line, ansi.Strip(line)))
	}
	return lines
}

func renderMarkdown(text string, width int) (string, error) {
	renderer, err := glamour.NewTermRenderer(
		glamour.WithStylePath("dark"),
		glamour.WithWordWrap(max(12, width)),
	)
	if err != nil {
		return "", err
	}
	return renderer.Render(text)
}
