package components

import (
	"strings"

	"charm.land/glamour/v2"
	glamouransi "charm.land/glamour/v2/ansi"
	"charm.land/glamour/v2/styles"
	"github.com/charmbracelet/x/ansi"
)

func markdownBodyLines(text string, width int) []renderedLine {
	return markdownBodyLinesWithBaseColor(text, width, themeColor(ColorText))
}

func markdownBodyLinesWithBaseColor(text string, width int, color string) []renderedLine {
	rendered, err := renderMarkdown(text, width)
	if err != nil || strings.TrimSpace(rendered) == "" {
		if color == themeColor(ColorTextStrong) {
			return styledWrappedBodyLines(text, width, assistantBodyStyle)
		}
		return wrappedBodyLines(text, width)
	}
	rendered = forceMarkdownBaseColor(rendered, color)
	rawLines := strings.Split(strings.TrimRight(rendered, "\n"), "\n")
	lines := make([]renderedLine, 0, len(rawLines))
	for _, line := range rawLines {
		lines = append(lines, bodyRenderedLine(line, ansi.Strip(line)))
	}
	return lines
}

func renderMarkdown(text string, width int) (string, error) {
	style := simplifiedMarkdownStyle()
	renderer, err := glamour.NewTermRenderer(
		glamour.WithStyles(style),
		glamour.WithWordWrap(max(12, width)),
	)
	if err != nil {
		return "", err
	}
	return renderer.Render(text)
}

func simplifiedMarkdownStyle() glamouransi.StyleConfig {
	style := styles.ASCIIStyleConfig
	baseColor := themeColor(ColorText)
	style.Document.StylePrimitive.Color = ptr(baseColor)
	style.Heading.StylePrimitive.Color = ptr(baseColor)
	style.Heading.StylePrimitive.Bold = ptr(true)
	style.Strong.Bold = ptr(true)
	style.Strong.BlockPrefix = ""
	style.Strong.BlockSuffix = ""
	style.Emph.Italic = ptr(true)
	style.Emph.BlockPrefix = ""
	style.Emph.BlockSuffix = ""
	style.Strikethrough.CrossedOut = ptr(true)
	style.Strikethrough.BlockPrefix = ""
	style.Strikethrough.BlockSuffix = ""
	style.Link.Color = ptr(baseColor)
	style.Link.Underline = ptr(true)
	style.LinkText.Color = ptr(baseColor)
	style.LinkText.Underline = ptr(true)
	style.Code.StylePrimitive.Color = ptr(baseColor)
	style.Code.StylePrimitive.BackgroundColor = nil
	style.CodeBlock.StylePrimitive.Color = ptr(baseColor)
	style.CodeBlock.Chroma = nil
	return style
}

func forceMarkdownBaseColor(rendered string, color string) string {
	defaultColor := themeColor(ColorText)
	if color == defaultColor {
		return rendered
	}
	return strings.ReplaceAll(rendered, "\x1b[38;5;"+defaultColor+"m", "\x1b[38;5;"+color+"m")
}

func ptr[T any](v T) *T {
	return &v
}
