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

type TranscriptCache struct {
	entries map[string]cachedMessage
}

type cachedMessage struct {
	width int
	role  viewmodel.Role
	title string
	body  string
	item  renderedMessage
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

func NewTranscriptCache() TranscriptCache {
	return TranscriptCache{entries: make(map[string]cachedMessage)}
}

func Transcript(width int, height int, messages []viewmodel.Message, scrollOffset int, hoveredID string, zones *zone.Manager) string {
	return TranscriptWithCache(width, height, messages, scrollOffset, hoveredID, zones, nil)
}

func TranscriptWithCache(width int, height int, messages []viewmodel.Message, scrollOffset int, hoveredID string, zones *zone.Manager, cache *TranscriptCache) string {
	panelHeight := max(4, height)
	innerWidth := max(20, width-2)
	contentWidth := max(12, innerWidth-2)
	innerHeight := max(1, panelHeight-2)
	visible := visibleMessages(messages, contentWidth, innerHeight, scrollOffset, cache)

	var body string
	if len(visible) == 0 {
		body = emptyStyle.Render("No transcript yet. Ask gode to inspect, edit, or run something.")
	} else {
		parts := make([]string, 0, len(visible))
		for _, item := range visible {
			block := strings.Join(item.lines, "\n")
			if item.id == hoveredID {
				block = messageHoverStyle.Width(contentWidth).Render(block)
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

func (c *TranscriptCache) Prune(messages []viewmodel.Message) {
	if c == nil || len(c.entries) == 0 {
		return
	}
	seen := make(map[string]struct{}, len(messages))
	for _, msg := range messages {
		seen[msg.ID] = struct{}{}
	}
	for id := range c.entries {
		if _, ok := seen[id]; !ok {
			delete(c.entries, id)
		}
	}
}

func visibleMessages(messages []viewmodel.Message, width int, height int, scrollOffset int, cache *TranscriptCache) []renderedMessage {
	if len(messages) == 0 || height <= 0 {
		return nil
	}

	startIndex := max(0, len(messages)-200)
	lineBudget := max(height, height+max(0, scrollOffset))
	reversed := make([]renderedMessage, 0, min(len(messages)-startIndex, height))
	total := 0
	for i := len(messages) - 1; i >= startIndex && total < lineBudget; i-- {
		item := renderMessageCached(messages[i], width, cache)
		reversed = append(reversed, item)
		total += len(item.lines)
	}
	if len(reversed) == 0 {
		return nil
	}

	rendered := make([]renderedMessage, len(reversed))
	for i := range reversed {
		rendered[len(reversed)-1-i] = reversed[i]
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

func renderMessageCached(msg viewmodel.Message, width int, cache *TranscriptCache) renderedMessage {
	if cache == nil {
		return renderMessage(msg, width)
	}
	if cache.entries == nil {
		cache.entries = make(map[string]cachedMessage)
	}
	if entry, ok := cache.entries[msg.ID]; ok &&
		entry.width == width &&
		entry.role == msg.Role &&
		entry.title == msg.Title &&
		entry.body == msg.Body {
		return entry.item
	}
	item := renderMessage(msg, width)
	cache.entries[msg.ID] = cachedMessage{
		width: width,
		role:  msg.Role,
		title: msg.Title,
		body:  msg.Body,
		item:  item,
	}
	return item
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
