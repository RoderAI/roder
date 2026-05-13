package agent

import (
	"context"
	"strings"

	"github.com/pandelisz/gode/internal/godex/provider"
	godeskills "github.com/pandelisz/gode/internal/godex/skills"
)

func (r *Runner) activeSkillSettings(ctx context.Context) (map[string]bool, error) {
	if r.loadActiveSkills != nil {
		settings, err := r.loadActiveSkills(ctx)
		if err != nil {
			return nil, err
		}
		return cloneActiveSkills(settings), nil
	}
	return cloneActiveSkills(r.activeSkills), nil
}

func cloneActiveSkills(activeSkills map[string]bool) map[string]bool {
	if activeSkills == nil {
		return nil
	}
	out := make(map[string]bool, len(activeSkills))
	for name, enabled := range activeSkills {
		out[name] = enabled
	}
	return out
}

func skillDiagnosticMessages(diagnostics []godeskills.Diagnostic) []provider.Message {
	if len(diagnostics) == 0 {
		return nil
	}
	messages := make([]provider.Message, 0, len(diagnostics))
	for _, diagnostic := range diagnostics {
		text := strings.TrimSpace(diagnostic.Message)
		if text == "" {
			continue
		}
		messages = append(messages, provider.Message{
			Role:    provider.RoleSystem,
			Content: "Skill diagnostic: " + text,
		})
	}
	return messages
}
