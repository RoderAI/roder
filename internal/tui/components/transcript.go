package components

import (
	"strings"

	"charm.land/lipgloss/v2"
	zone "github.com/lrstanley/bubblezone/v2"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

type renderedMessage struct {
	id    string
	lines []string
}

var (
	transcriptStyle = lipgloss.NewStyle().
			Padding(1, 1)
	emptyStyle = lipgloss.NewStyle().
			Foreground(lipgloss.Color("244")).
			Italic(true)
	messageHoverStyle = lipgloss.NewStyle().
				Foreground(lipgloss.Color("231"))
	labelStyle = lipgloss.NewStyle().
			Bold(true).
			Padding(0, 1)
	bodyStyle = lipgloss.NewStyle().
			Foreground(lipgloss.Color("252"))
)

func Transcript(width int, height int, messages []viewmodel.Message, scrollOffset int, hoveredID string, zones *zone.Manager) string {
	panelHeight := max(4, height)
	innerWidth := max(20, width-2)
	innerHeight := max(1, panelHeight-2)
	visible := visibleMessages(messages, innerWidth, innerHeight, scrollOffset)

	var body string
	if len(visible) == 0 {
		body = emptyStyle.Render("No transcript yet. Ask gode to inspect, edit, or run something.")
	} else {
		parts := make([]string, 0, len(visible))
		for _, item := range visible {
			block := strings.Join(item.lines, "\n")
			if item.id == hoveredID {
				block = messageHoverStyle.Width(innerWidth).Render(block)
			}
			parts = append(parts, zones.Mark(viewmodel.MessageZoneID(item.id), block))
		}
		body = strings.Join(parts, "\n")
	}

	panel := transcriptStyle.
		Width(innerWidth).
		Height(panelHeight).
		Render(body)
	return zones.Mark(viewmodel.TranscriptZoneID, panel)
}

func visibleMessages(messages []viewmodel.Message, width int, height int, scrollOffset int) []renderedMessage {
	if len(messages) == 0 || height <= 0 {
		return nil
	}

	rendered := make([]renderedMessage, 0, min(len(messages), 200))
	for i := max(0, len(messages)-200); i < len(messages); i++ {
		rendered = append(rendered, renderMessage(messages[i], width))
	}

	total := 0
	for _, item := range rendered {
		total += len(item.lines)
	}
	scrollOffset = clamp(scrollOffset, 0, max(0, total-height))
	startLine := max(0, total-height-scrollOffset)
	endLine := min(total, startLine+height)

	visible := make([]renderedMessage, 0, len(rendered))
	cursor := 0
	for _, item := range rendered {
		itemStart := cursor
		itemEnd := cursor + len(item.lines)
		cursor = itemEnd

		if itemEnd <= startLine || itemStart >= endLine {
			continue
		}

		from := max(0, startLine-itemStart)
		to := min(len(item.lines), endLine-itemStart)
		visible = append(visible, renderedMessage{id: item.id, lines: item.lines[from:to]})
	}
	return visible
}

func renderMessage(msg viewmodel.Message, width int) renderedMessage {
	label := roleLabelStyle(msg.Role).Render(strings.ToUpper(string(msg.Role)))
	if msg.Title != "" {
		label += " " + lipgloss.NewStyle().Foreground(lipgloss.Color("246")).Render(msg.Title)
	}

	bodyWidth := max(12, width-2)
	lines := []string{label}
	for _, line := range wrapText(msg.Body, bodyWidth) {
		lines = append(lines, "  "+bodyStyle.Render(line))
	}
	return renderedMessage{id: msg.ID, lines: lines}
}

func roleLabelStyle(role viewmodel.Role) lipgloss.Style {
	style := labelStyle.Copy()
	switch role {
	case viewmodel.RoleUser:
		return style.Foreground(lipgloss.Color("16")).Background(lipgloss.Color("212"))
	case viewmodel.RoleAssistant:
		return style.Foreground(lipgloss.Color("16")).Background(lipgloss.Color("86"))
	case viewmodel.RoleTool:
		return style.Foreground(lipgloss.Color("16")).Background(lipgloss.Color("111"))
	case viewmodel.RoleError:
		return style.Foreground(lipgloss.Color("231")).Background(lipgloss.Color("160"))
	default:
		return style.Foreground(lipgloss.Color("231")).Background(lipgloss.Color("240"))
	}
}

func wrapText(text string, width int) []string {
	text = strings.TrimSpace(text)
	if text == "" {
		return []string{""}
	}

	var out []string
	for _, raw := range strings.Split(text, "\n") {
		words := strings.Fields(raw)
		if len(words) == 0 {
			out = append(out, "")
			continue
		}

		line := ""
		for _, word := range words {
			if lipgloss.Width(word) > width {
				if line != "" {
					out = append(out, line)
					line = ""
				}
				out = append(out, splitLongWord(word, width)...)
				continue
			}
			if line == "" {
				line = word
				continue
			}
			next := line + " " + word
			if lipgloss.Width(next) > width {
				out = append(out, line)
				line = word
				continue
			}
			line = next
		}
		if line != "" {
			out = append(out, line)
		}
	}
	return out
}

func splitLongWord(word string, width int) []string {
	var out []string
	var line string
	for _, r := range word {
		next := line + string(r)
		if line != "" && lipgloss.Width(next) > width {
			out = append(out, line)
			line = string(r)
			continue
		}
		line = next
	}
	if line != "" {
		out = append(out, line)
	}
	return out
}

func clamp(v int, low int, high int) int {
	if high < low {
		return low
	}
	if v < low {
		return low
	}
	if v > high {
		return high
	}
	return v
}

func min(a, b int) int {
	if a < b {
		return a
	}
	return b
}
