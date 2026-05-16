package roadmap

import (
	"fmt"
	"path/filepath"
	"strings"
)

func Validate(doc Document) []Diagnostic {
	var diagnostics []Diagnostic
	add := func(line int, message string) {
		diagnostics = append(diagnostics, Diagnostic{Path: doc.Path, Line: line, Severity: "error", Message: message})
	}
	clean := filepath.ToSlash(doc.Path)
	if clean != "" && !strings.Contains(clean, "/roadmap/") && !strings.HasPrefix(clean, "roadmap/") {
		add(0, "roadmap path must be under roadmap/")
	}
	if strings.TrimSpace(doc.Goal) == "" {
		add(0, "missing goal")
	}
	if strings.TrimSpace(doc.Architecture) == "" {
		add(0, "missing architecture")
	}
	if !containsLine(doc.Lines, "## Owned Paths") {
		add(0, "missing owned paths")
	}
	if !containsLine(doc.Lines, "## Tasks") {
		add(0, "missing tasks section")
	}
	if len(doc.Tasks) == 0 {
		add(0, "missing task checkboxes")
	}
	if len(doc.RunBlocks) == 0 || !containsLinePrefix(doc.Lines, "Run:") {
		add(0, "missing run command")
	}
	if len(doc.AcceptanceSections) == 0 {
		add(0, "missing acceptance criteria")
	}
	seen := map[string]int{}
	for _, task := range doc.Tasks {
		if first, ok := seen[task.ID]; ok {
			add(task.Line, fmt.Sprintf("duplicate task id %s first seen on line %d", task.ID, first))
			continue
		}
		seen[task.ID] = task.Line
	}
	return diagnostics
}

func containsLine(lines []string, value string) bool {
	for _, line := range lines {
		if strings.TrimSpace(line) == value {
			return true
		}
	}
	return false
}

func containsLinePrefix(lines []string, prefix string) bool {
	for _, line := range lines {
		if strings.HasPrefix(strings.TrimSpace(line), prefix) {
			return true
		}
	}
	return false
}
