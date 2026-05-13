package tui

import "strings"

func summarizeToolTimeline(tool string, input map[string]any, output string) string {
	switch tool {
	case "read_file":
		path := strings.TrimSpace(inputString(input, "path"))
		if path == "" {
			return "read file"
		}
		return "read " + path
	default:
		return truncate(output, 1600)
	}
}

func inputString(input map[string]any, key string) string {
	if input == nil {
		return ""
	}
	switch value := input[key].(type) {
	case string:
		return value
	case []byte:
		return string(value)
	default:
		return ""
	}
}
