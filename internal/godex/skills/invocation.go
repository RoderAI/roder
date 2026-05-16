package skills

import (
	"fmt"
	"html"
	"regexp"
	"strings"

	"github.com/pandelisz/gode/internal/godex/provider"
)

var (
	invocationPattern     = regexp.MustCompile(`\$([a-zA-Z0-9]+(?:-[a-zA-Z0-9]+)*)`)
	namedSkillLinkPattern = regexp.MustCompile(`\[\$([a-zA-Z0-9]+(?:-[a-zA-Z0-9]+)*)\]\(([^)]+)\)`)
	skillPathLinkPattern  = regexp.MustCompile(`\[[^\]]+\]\(([^)]+SKILL\.md)\)`)
	rawSkillPathPattern   = regexp.MustCompile(`(?:skill://|file://)?(/[^\s]+/SKILL\.md)`)
)

type InvocationResult struct {
	Prompt      string
	Messages    []provider.Message
	Invoked     []Skill
	Diagnostics []Diagnostic
}

type InvocationSelection struct {
	Name string
	Path string
}

func ApplyInvocations(prompt string, catalog Catalog) InvocationResult {
	return ApplyInvocationsWithConfig(prompt, catalog, Config{})
}

func ApplyInvocationsWithConfig(prompt string, catalog Catalog, config Config) InvocationResult {
	return ApplyInvocationsWithSelections(prompt, catalog, config, nil)
}

func ApplyInvocationsWithSelections(prompt string, catalog Catalog, config Config, selections []InvocationSelection) InvocationResult {
	disabled := DisabledSkillPaths(catalog.Skills, config)
	byName := map[string][]Skill{}
	byPath := map[string]Skill{}
	for _, skill := range catalog.Skills {
		path := skillIdentity(skill)
		if _, ok := disabled[path]; ok {
			continue
		}
		byName[skill.Name] = append(byName[skill.Name], skill)
		byPath[path] = skill
	}

	selected := map[string]Skill{}
	selectByName := func(name string) bool {
		matches := byName[strings.TrimSpace(name)]
		if len(matches) != 1 {
			return false
		}
		selected[skillIdentity(matches[0])] = matches[0]
		return true
	}
	selectByPath := func(path string) bool {
		path = normalizeInvocationPath(path)
		skill, ok := byPath[canonicalPath(path)]
		if !ok {
			return false
		}
		selected[skillIdentity(skill)] = skill
		return true
	}
	for _, selection := range selections {
		switch {
		case strings.TrimSpace(selection.Path) != "":
			selectByPath(selection.Path)
		case strings.TrimSpace(selection.Name) != "":
			selectByName(selection.Name)
		}
	}

	cleaned := namedSkillLinkPattern.ReplaceAllStringFunc(prompt, func(match string) string {
		parts := namedSkillLinkPattern.FindStringSubmatch(match)
		if len(parts) != 3 {
			return match
		}
		if !selectByPath(parts[2]) {
			return match
		}
		return ""
	})
	cleaned = skillPathLinkPattern.ReplaceAllStringFunc(cleaned, func(match string) string {
		parts := skillPathLinkPattern.FindStringSubmatch(match)
		if len(parts) != 2 || !selectByPath(parts[1]) {
			return match
		}
		return ""
	})
	cleaned = rawSkillPathPattern.ReplaceAllStringFunc(cleaned, func(match string) string {
		if !selectByPath(match) {
			return match
		}
		return ""
	})
	cleaned = invocationPattern.ReplaceAllStringFunc(cleaned, func(match string) string {
		name := strings.TrimPrefix(match, "$")
		if !selectByName(name) {
			return match
		}
		return ""
	})

	result := InvocationResult{
		Prompt:      prompt,
		Diagnostics: append([]Diagnostic(nil), catalog.Diagnostics...),
	}
	if len(selected) > 0 {
		result.Prompt = strings.Join(strings.Fields(cleaned), " ")
		if strings.TrimSpace(result.Prompt) == "" {
			result.Prompt = strings.TrimSpace(cleaned)
		}
	}
	for _, skill := range catalog.Skills {
		if invoked, ok := selected[skillIdentity(skill)]; ok {
			result.Invoked = append(result.Invoked, invoked)
			result.Messages = append(result.Messages, skillMessage(invoked))
		}
	}
	return result
}

func normalizeInvocationPath(path string) string {
	path = strings.TrimSpace(path)
	path = strings.TrimPrefix(path, "skill://")
	path = strings.TrimPrefix(path, "file://")
	return path
}

func DisabledMentionDiagnostics(prompt string, catalog Catalog, config Config) []Diagnostic {
	disabled := DisabledSkillPaths(catalog.Skills, config)
	var diagnostics []Diagnostic
	for _, match := range invocationPattern.FindAllStringSubmatch(prompt, -1) {
		if len(match) != 2 {
			continue
		}
		for _, skill := range catalog.Skills {
			if skill.Name != match[1] {
				continue
			}
			if _, ok := disabled[skillIdentity(skill)]; ok {
				diagnostics = append(diagnostics, Diagnostic{
					Path:    skill.Path,
					Message: fmt.Sprintf("skill %q is disabled; enable it in settings before invoking", skill.Name),
				})
			}
		}
	}
	return diagnostics
}

func skillMessage(skill Skill) provider.Message {
	content := skill.Content
	if strings.TrimSpace(content) == "" {
		content = skill.Body
	}
	return provider.Message{
		Role: provider.RoleUser,
		Content: fmt.Sprintf(
			"<skill>\n<name>%s</name>\n<path>%s</path>\n%s\n</skill>",
			html.EscapeString(skill.Name),
			html.EscapeString(skill.Path),
			strings.TrimSpace(content),
		),
	}
}
