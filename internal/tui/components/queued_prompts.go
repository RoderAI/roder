package components

import (
	"fmt"
	"strings"

	"charm.land/lipgloss/v2"
)

const maxQueuedPromptRows = 6

var (
	queuedTitleStyle lipgloss.Style
	queuedItemStyle  lipgloss.Style
)

func resetQueuedPromptStyles() {
	queuedTitleStyle = lipgloss.NewStyle().
		Foreground(ThemeColor(ColorTextStrong)).
		Bold(true)
	queuedItemStyle = lipgloss.NewStyle().
		Foreground(ThemeColor(ColorSubtle)).
		Italic(true)
}

func QueuedPromptsHeight(prompts []string) int {
	if len(prompts) == 0 {
		return 0
	}
	return 1 + min(len(prompts), maxQueuedPromptRows)
}

func QueuedPrompts(width int, prompts []string) string {
	if len(prompts) == 0 {
		return ""
	}
	contentWidth := max(12, width-4)
	rows := []string{queuedTitleStyle.Render("• Queued follow-up inputs")}
	visibleCount := min(len(prompts), maxQueuedPromptRows)
	for i := 0; i < visibleCount; i++ {
		rows = append(rows, queuedItemStyle.Render("↳ "+truncateCell(strings.TrimSpace(prompts[i]), max(8, contentWidth-2))))
	}
	if len(prompts) > visibleCount {
		remaining := len(prompts) - visibleCount
		rows[len(rows)-1] = queuedItemStyle.Render("↳ ... " + queuedOverflowLabel(remaining))
	}
	return lipgloss.NewStyle().
		Width(max(20, width-2)).
		Padding(0, 1).
		Render(strings.Join(rows, "\n"))
}

func queuedOverflowLabel(count int) string {
	if count == 1 {
		return "1 more queued input"
	}
	return fmt.Sprintf("%d more queued inputs", count)
}
