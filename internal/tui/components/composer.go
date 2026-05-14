package components

import (
	"image/color"
	"strings"

	"charm.land/lipgloss/v2"
	zone "github.com/lrstanley/bubblezone/v2"
	"github.com/pandelisz/gode/internal/tui/selection"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

var composerStyle lipgloss.Style

func resetComposerStyles() {
	composerStyle = lipgloss.NewStyle().
		Border(lipgloss.NormalBorder(), true).
		Padding(0, 1)
}

func Composer(width int, input string) string {
	return ComposerWithSelection(width, input, ComposerOptions{}, nil)
}

type ComposerOptions struct {
	Value          string
	Selection      selection.OffsetRange
	SelectionStyle lipgloss.Style
	AutoApprove    bool
}

func ComposerWithSelection(width int, input string, options ComposerOptions, zones *zone.Manager) string {
	body := input
	if options.Selection.Active {
		body = renderSelectedComposerValue(options.Value, options.Selection, options.SelectionStyle)
	}
	out := composerStyle.
		BorderForeground(composerBorderColor(options.AutoApprove)).
		Width(max(20, width-2)).
		Render(body)
	if zones != nil {
		return zones.Mark(viewmodel.ComposerZoneID, out)
	}
	return out
}

func ComposerSelectionText(value string, selected selection.OffsetRange) string {
	return selected.SelectedText(value)
}

func renderSelectedComposerValue(value string, selected selection.OffsetRange, style lipgloss.Style) string {
	if value == "" {
		return ""
	}
	if style.GetBackground() == nil && style.GetForeground() == nil {
		style = ThemeSelectionStyle()
	}
	start, end, ok := selected.Normalize(value)
	if !ok {
		return value
	}
	runes := []rune(value)
	var out strings.Builder
	out.WriteString(string(runes[:start]))
	out.WriteString(style.Render(string(runes[start:end])))
	out.WriteString(string(runes[end:]))
	return out.String()
}

func composerBorderColor(autoApprove bool) color.Color {
	if autoApprove {
		return ThemeColor(ColorAccent)
	}
	return ThemeColor(ColorBorder)
}

func ComposerVisualLines(value string, width int) []string {
	wrapWidth := composerWrapWidth(width)
	if value == "" {
		return []string{""}
	}
	var lines []string
	for _, raw := range strings.Split(value, "\n") {
		runes := []rune(raw)
		if len(runes) == 0 {
			lines = append(lines, "")
			continue
		}
		for len(runes) > wrapWidth {
			lines = append(lines, string(runes[:wrapWidth]))
			runes = runes[wrapWidth:]
		}
		lines = append(lines, string(runes))
	}
	return lines
}

func ComposerOffsetAt(value string, width int, row int, col int) int {
	lines := ComposerVisualLines(value, width)
	row = clamp(row, 0, len(lines)-1)
	col = max(0, col)
	offset := 0
	for i := 0; i < row; i++ {
		offset += len([]rune(lines[i]))
	}
	return offset + min(col, len([]rune(lines[row])))
}

func composerWrapWidth(width int) int {
	return max(1, width-4)
}
