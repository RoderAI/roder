package tui

import (
	"strings"

	"charm.land/lipgloss/v2"
	"github.com/pandelisz/gode/internal/tui/components"
)

func (m *Model) scrollBy(delta int) {
	m.scrollOffset = clamp(m.scrollOffset+delta, 0, m.maxScrollOffset())
	m.followTail = m.scrollOffset == 0
}

func (m *Model) scrollToOldest() {
	m.scrollOffset = m.maxScrollOffset()
	m.followTail = false
}

func (m *Model) follow() {
	m.scrollOffset = 0
	m.followTail = true
}

func (m *Model) clampScroll() {
	m.scrollOffset = clamp(m.scrollOffset, 0, m.maxScrollOffset())
	if m.scrollOffset == 0 {
		m.followTail = true
	}
}

func (m *Model) maxScrollOffset() int {
	return max(0, m.cachedTranscriptLineCount()-m.visibleTranscriptLines())
}

func (m *Model) visibleTranscriptLines() int {
	composerHeight := max(3, m.input.Height()+2)
	reasoningHeight := components.ReasoningSummaryHeight(m.visibleReasoningSummary(), m.height)
	transcriptHeight := max(6, m.height-composerHeight-reasoningHeight-3)
	return max(1, transcriptHeight-2)
}

func (m *Model) cachedTranscriptLineCount() int {
	width := m.transcriptWrapWidth()
	if !m.transcriptLineDirty && m.transcriptLineWidth == width {
		return m.transcriptLineTotal
	}

	total := 0
	for _, msg := range m.messages {
		total++
		total += countWrappedLines(msg.Body, width)
	}
	m.transcriptLineWidth = width
	m.transcriptLineTotal = total
	m.transcriptLineDirty = false
	return total
}

func (m *Model) markTranscriptLinesDirty() {
	m.transcriptLineDirty = true
}

func (m *Model) transcriptWrapWidth() int {
	return max(12, m.width-4)
}

func countWrappedLines(text string, width int) int {
	text = strings.TrimSpace(text)
	if text == "" {
		return 1
	}

	total := 0
	for _, raw := range strings.Split(text, "\n") {
		words := strings.Fields(raw)
		if len(words) == 0 {
			total++
			continue
		}

		line := ""
		for _, word := range words {
			if lipgloss.Width(word) > width {
				if line != "" {
					total++
					line = ""
				}
				total += longWordLines(word, width)
				continue
			}
			if line == "" {
				line = word
				continue
			}
			next := line + " " + word
			if lipgloss.Width(next) > width {
				total++
				line = word
				continue
			}
			line = next
		}
		if line != "" {
			total++
		}
	}
	return total
}

func longWordLines(word string, width int) int {
	lines := 1
	line := ""
	for _, r := range word {
		next := line + string(r)
		if line != "" && lipgloss.Width(next) > width {
			lines++
			line = string(r)
			continue
		}
		line = next
	}
	return lines
}
