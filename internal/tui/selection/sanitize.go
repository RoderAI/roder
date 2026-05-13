package selection

import (
	"strings"

	"github.com/charmbracelet/x/ansi"
)

type TranscriptLineRef struct {
	MessageIndex int
	LogicalLine  int
	DisplayLine  int
	Text         string
	CopyText     string
	Decorative   bool
}

func SanitizeTranscriptCopy(lines []TranscriptLineRef) string {
	clean := make([]string, 0, len(lines))
	for _, line := range lines {
		if line.Decorative {
			continue
		}
		text := line.CopyText
		if text == "" {
			text = line.Text
		}
		text = strings.TrimRight(ansi.Strip(text), " \t")
		if isDecorativeTranscriptLine(text) {
			continue
		}
		clean = append(clean, text)
	}

	start := 0
	for start < len(clean) && strings.TrimSpace(clean[start]) == "" {
		start++
	}
	end := len(clean)
	for end > start && strings.TrimSpace(clean[end-1]) == "" {
		end--
	}
	return strings.Join(clean[start:end], "\n")
}

func isDecorativeTranscriptLine(text string) bool {
	trimmed := strings.TrimSpace(text)
	if trimmed == "" {
		return false
	}
	if isRoleLabel(trimmed) {
		return true
	}
	if isBorderLine(trimmed) {
		return true
	}
	lower := strings.ToLower(trimmed)
	if strings.HasPrefix(trimmed, "...") && strings.Contains(lower, "lines collapsed") {
		return true
	}
	if strings.Contains(lower, "ctrl+") && (strings.Contains(lower, "copy") || strings.Contains(lower, "errors") || strings.Contains(lower, "close")) {
		return true
	}
	if strings.Contains(lower, "scroll") && (strings.Contains(lower, "errors") || strings.Contains(lower, "ctx ")) {
		return true
	}
	return false
}

func isRoleLabel(text string) bool {
	switch strings.ToUpper(text) {
	case "USER", "ASSISTANT", "SYSTEM", "TOOL", "ERROR":
		return true
	default:
		return false
	}
}

func isBorderLine(text string) bool {
	hasBorder := false
	for _, char := range text {
		switch char {
		case ' ', '\t', '┌', '┐', '└', '┘', '│', '─', '├', '┤', '┬', '┴', '┼', '╭', '╮', '╰', '╯', '═', '╞', '╡', '╪':
			if char != ' ' && char != '\t' {
				hasBorder = true
			}
		default:
			return false
		}
	}
	return hasBorder
}
