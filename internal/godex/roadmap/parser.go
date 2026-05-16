package roadmap

import (
	"fmt"
	"hash/fnv"
	"os"
	"regexp"
	"strings"
)

var checkboxLine = regexp.MustCompile(`^(\s*[-*]\s+\[)( |x|X)(\]\s+)(.*)$`)

func ParseFile(path string) (Document, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return Document{}, fmt.Errorf("read roadmap %s: %w", path, err)
	}
	return Parse(path, string(data))
}

func Parse(path string, raw string) (Document, error) {
	doc := Document{Path: path, Raw: raw, Lines: splitLines(raw)}
	inCode := false
	for i, line := range doc.Lines {
		lineNo := i + 1
		trimmed := strings.TrimSpace(line)
		if strings.HasPrefix(trimmed, "```") {
			if inCode {
				inCode = false
			} else {
				inCode = true
				doc.RunBlocks = append(doc.RunBlocks, RunBlock{Line: lineNo, Text: trimmed})
			}
			continue
		}
		if inCode {
			continue
		}
		switch {
		case doc.Title == "" && strings.HasPrefix(trimmed, "# "):
			doc.Title = strings.TrimSpace(strings.TrimPrefix(trimmed, "# "))
		case strings.HasPrefix(trimmed, "**Goal:**"):
			doc.Goal = strings.TrimSpace(strings.TrimPrefix(trimmed, "**Goal:**"))
		case strings.HasPrefix(trimmed, "**Architecture:**"):
			doc.Architecture = strings.TrimSpace(strings.TrimPrefix(trimmed, "**Architecture:**"))
		case strings.HasPrefix(trimmed, "**Tech Stack:**"):
			doc.TechStack = strings.TrimSpace(strings.TrimPrefix(trimmed, "**Tech Stack:**"))
		case strings.EqualFold(trimmed, "Acceptance:") || strings.HasSuffix(trimmed, " Acceptance") || strings.HasPrefix(trimmed, "## Phase Acceptance"):
			doc.AcceptanceSections = append(doc.AcceptanceSections, Section{Line: lineNo, Title: strings.Trim(trimmed, "# ")})
		}
		if match := checkboxLine.FindStringSubmatch(line); len(match) == 5 {
			text := strings.TrimSpace(match[4])
			doc.Tasks = append(doc.Tasks, Task{
				ID:      TaskID(text),
				Text:    text,
				Checked: strings.EqualFold(match[2], "x"),
				Line:    lineNo,
			})
		}
	}
	return doc, nil
}

func TaskID(text string) string {
	normalized := strings.Join(strings.Fields(strings.ToLower(text)), " ")
	h := fnv.New32a()
	_, _ = h.Write([]byte(normalized))
	return fmt.Sprintf("task-%08x", h.Sum32())
}

func splitLines(raw string) []string {
	raw = strings.ReplaceAll(raw, "\r\n", "\n")
	if raw == "" {
		return nil
	}
	lines := strings.Split(raw, "\n")
	if lines[len(lines)-1] == "" {
		lines = lines[:len(lines)-1]
	}
	return lines
}
