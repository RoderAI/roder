package components

import (
	"strings"

	"charm.land/lipgloss/v2"
)

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
				if keepWrappedWordTogether(word) {
					out = append(out, word)
					continue
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

func keepWrappedWordTogether(word string) bool {
	return strings.Contains(word, "://") || strings.Contains(word, "](http://") || strings.Contains(word, "](https://")
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
