package skills

import (
	"fmt"
	"html"
	"regexp"
	"strings"

	"github.com/pandelisz/gode/internal/godex/provider"
)

var invocationPattern = regexp.MustCompile(`\$([a-zA-Z0-9]+(?:-[a-zA-Z0-9]+)*)`)

type InvocationResult struct {
	Prompt      string
	Messages    []provider.Message
	Invoked     []Skill
	Diagnostics []Diagnostic
}

func ApplyInvocations(prompt string, catalog Catalog) InvocationResult {
	byName := map[string]Skill{}
	for _, skill := range catalog.Skills {
		byName[skill.Name] = skill
	}
	invoked := map[string]Skill{}
	found := false
	cleaned := invocationPattern.ReplaceAllStringFunc(prompt, func(match string) string {
		name := strings.TrimPrefix(match, "$")
		skill, ok := byName[name]
		if !ok {
			return match
		}
		found = true
		invoked[name] = skill
		return ""
	})

	result := InvocationResult{
		Prompt:      prompt,
		Diagnostics: append([]Diagnostic(nil), catalog.Diagnostics...),
	}
	if found {
		result.Prompt = strings.Join(strings.Fields(cleaned), " ")
		if strings.TrimSpace(result.Prompt) == "" {
			result.Prompt = strings.TrimSpace(cleaned)
		}
	}
	for _, skill := range catalog.Skills {
		if invokedSkill, ok := invoked[skill.Name]; ok {
			result.Invoked = append(result.Invoked, invokedSkill)
			result.Messages = append(result.Messages, skillMessage(invokedSkill))
		}
	}
	return result
}

func skillMessage(skill Skill) provider.Message {
	return provider.Message{
		Role: provider.RoleSystem,
		Content: fmt.Sprintf(
			"<skill name=\"%s\">\n%s\n</skill>",
			html.EscapeString(skill.Name),
			strings.TrimSpace(skill.Body),
		),
	}
}
