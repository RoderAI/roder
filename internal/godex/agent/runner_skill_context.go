package agent

import (
	"context"
	"strings"

	"github.com/pandelisz/gode/internal/godex/provider"
	godeskills "github.com/pandelisz/gode/internal/godex/skills"
)

func (r *Runner) skillContextMessages(ctx context.Context, prompt string) ([]provider.Message, string, error) {
	skillsConfig, err := r.activeSkillSettings(ctx)
	if err != nil {
		return nil, "", err
	}
	skillCatalog := godeskills.Catalog{Skills: r.skills}
	available := godeskills.RenderAvailable(skillCatalog, godeskills.RenderOptions{Config: skillsConfig})

	messages := []provider.Message{}
	if strings.TrimSpace(available.Body) != "" {
		messages = append(messages, provider.Message{
			Role:    provider.RoleSystem,
			Content: godeskills.SkillsInstructionsOpenTag + available.Body + godeskills.SkillsInstructionsCloseTag,
		})
	}
	if available.Report.Warning != "" {
		messages = append(messages, provider.Message{Role: provider.RoleSystem, Content: "Skill warning: " + available.Report.Warning})
	}

	invocation := godeskills.ApplyInvocationsWithConfig(prompt, skillCatalog, skillsConfig)
	invocation.Diagnostics = append(invocation.Diagnostics, godeskills.DisabledMentionDiagnostics(prompt, skillCatalog, skillsConfig)...)
	messages = append(messages, invocation.Messages...)
	messages = append(messages, skillDiagnosticMessages(invocation.Diagnostics)...)
	return messages, invocation.Prompt, nil
}
