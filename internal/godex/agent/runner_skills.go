package agent

import (
	"context"
	"strings"

	"github.com/pandelisz/gode/internal/godex/provider"
	godeskills "github.com/pandelisz/gode/internal/godex/skills"
)

func (r *Runner) activeSkillSettings(ctx context.Context) (godeskills.Config, error) {
	if r.loadSkillsConfig != nil {
		settings, err := r.loadSkillsConfig(ctx)
		if err != nil {
			return godeskills.Config{}, err
		}
		return settings, nil
	}
	return r.skillsConfig, nil
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
