package agent

import (
	"context"
	"fmt"
	"html"
	"strings"

	"github.com/pandelisz/gode/internal/godex/provider"
	godeskills "github.com/pandelisz/gode/internal/godex/skills"
)

func (r *Runner) implicitSkillMessages(ctx context.Context, toolCalls []pendingToolCall) ([]provider.Message, []provider.Item, error) {
	if len(toolCalls) == 0 || len(r.skills) == 0 {
		return nil, nil, nil
	}
	config, err := r.activeSkillSettings(ctx)
	if err != nil {
		return nil, nil, err
	}
	catalog := godeskills.Catalog{Skills: r.skills}
	seen := map[string]struct{}{}
	var messages []provider.Message
	for _, toolCall := range toolCalls {
		command := implicitCommandFromTool(toolCall.request)
		if command == "" {
			continue
		}
		skill, ok := godeskills.DetectImplicitInvocation(catalog, config, command, r.workspace)
		if !ok {
			continue
		}
		id := godeskills.SkillIdentity(skill)
		if _, ok := seen[id]; ok {
			continue
		}
		seen[id] = struct{}{}
		messages = append(messages, implicitSkillMessage(skill, toolCall.request))
	}
	if len(messages) == 0 {
		return nil, nil, nil
	}
	items := make([]provider.Item, 0, len(messages))
	for _, message := range messages {
		items = append(items, providerItemFromProviderMessage(message))
	}
	return messages, items, nil
}

func implicitCommandFromTool(request *provider.ToolRequest) string {
	if request == nil {
		return ""
	}
	switch request.Name {
	case "shell":
		return stringInput(request.Input, "command")
	case "read_file":
		path := stringInput(request.Input, "path")
		if path == "" {
			return ""
		}
		return "cat " + shellQuote(path)
	default:
		return ""
	}
}

func implicitSkillMessage(skill godeskills.Skill, request *provider.ToolRequest) provider.Message {
	return provider.Message{
		Role: provider.RoleUser,
		Content: fmt.Sprintf(
			"Implicit skill context selected after tool %q.\n<skill>\n<name>%s</name>\n<path>%s</path>\n%s\n</skill>",
			request.Name,
			html.EscapeString(skill.Name),
			html.EscapeString(skill.Path),
			strings.TrimSpace(firstNonEmpty(skill.Content, skill.Body)),
		),
	}
}

func stringInput(input map[string]any, key string) string {
	value, _ := input[key].(string)
	return strings.TrimSpace(value)
}

func shellQuote(value string) string {
	if value == "" {
		return "''"
	}
	return "'" + strings.ReplaceAll(value, "'", "'\\''") + "'"
}
