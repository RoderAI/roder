package tui

import (
	"fmt"
	"strings"
	"time"

	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

func (m *Model) addErrorLog(source string, message string) {
	message = strings.TrimSpace(message)
	if message == "" {
		return
	}
	source = strings.TrimSpace(source)
	if source == "" {
		source = "run"
	}
	m.errorLog = append(m.errorLog, viewmodel.ErrorLogEntry{
		ID:      fmt.Sprintf("e%d", len(m.errorLog)+1),
		Time:    time.Now().Format("15:04:05"),
		Source:  source,
		Message: message,
	})
	if overflow := len(m.errorLog) - maxErrorLogEntries; overflow > 0 {
		m.errorLog = m.errorLog[overflow:]
	}
}

func (m Model) hasRecentError(message string) bool {
	message = strings.TrimSpace(message)
	if message == "" {
		return false
	}
	for i := len(m.errorLog) - 1; i >= 0 && i >= len(m.errorLog)-3; i-- {
		entry := m.errorLog[i]
		if strings.TrimSpace(entry.Message) == message {
			return true
		}
	}
	return false
}

func summarizeTimelineError(message string) string {
	message = strings.TrimSpace(message)
	if message == "" {
		return ""
	}
	lines := nonEmptyLines(message)
	first := lines[0]
	if status := valueLine(lines, "status: "); status != "" {
		if first == "OpenAI stream request failed" {
			if apiMessage := valueLine(lines, "error_message: "); apiMessage != "" {
				return truncateSingleLine(first+": "+status+" - "+apiMessage+" - ctrl+l for details", 180)
			}
			return truncateSingleLine(first+": "+status+" - ctrl+l for details", 180)
		}
		return truncateSingleLine(status+" - ctrl+l for details", 180)
	}
	if len(lines) > 1 {
		return truncateSingleLine(first+" - ctrl+l for details", 180)
	}
	return truncateSingleLine(first, 180)
}

func nonEmptyLines(message string) []string {
	lines := []string{}
	for _, line := range strings.Split(message, "\n") {
		line = strings.TrimSpace(line)
		if line != "" {
			lines = append(lines, line)
		}
	}
	if len(lines) == 0 {
		return []string{""}
	}
	return lines
}

func valueLine(lines []string, prefix string) string {
	for _, line := range lines {
		if strings.HasPrefix(line, prefix) {
			return strings.TrimSpace(strings.TrimPrefix(line, prefix))
		}
	}
	return ""
}

func truncateSingleLine(text string, limit int) string {
	text = strings.Join(strings.Fields(text), " ")
	if len(text) <= limit {
		return text
	}
	return text[:limit] + "..."
}
