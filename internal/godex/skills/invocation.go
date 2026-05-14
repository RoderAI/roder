package skills

import (
	"fmt"
	"html"
	"regexp"
	"strings"

	"github.com/pandelisz/gode/internal/godex/provider"
)

var (
	invocationPattern = regexp.MustCompile(`\$([a-zA-Z0-9]+(?:-[a-zA-Z0-9]+)*)`)
	skillLinkPattern  = regexp.MustCompile(`\[\$([a-zA-Z0-9]+(?:-[a-zA-Z0-9]+)*)\]\(([^)]+)\)`)
)

type InvocationResult struct {
	Prompt      string
	Messages    []provider.Message
	Invoked     []Skill
	Diagnostics []Diagnostic
}

func ApplyInvocations(prompt string, catalog Catalog) InvocationResult {
	return ApplyInvocationsWithConfig(prompt, catalog, Config{})
}

func ApplyInvocationsWithConfig(prompt string, catalog Catalog, config Config) InvocationResult {
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
	cleaned := skillLinkPattern.ReplaceAllStringFunc(prompt, func(match string) string {
		parts := skillLinkPattern.FindStringSubmatch(match)
		if len(parts) != 3 {
			return match
		}
		path := strings.TrimPrefix(strings.TrimSpace(parts[2]), "skill://")
		skill, ok := byPath[canonicalPath(path)]
		if !ok {
			return match
		}
		selected[skillIdentity(skill)] = skill
		return ""
	})
	cleaned = invocationPattern.ReplaceAllStringFunc(cleaned, func(match string) string {
		name := strings.TrimPrefix(match, "$")
		matches := byName[name]
		if len(matches) != 1 {
			return match
		}
		selected[skillIdentity(matches[0])] = matches[0]
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
