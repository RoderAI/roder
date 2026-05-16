package roadmap

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

const planningSkillPath = ".agents/skills/roadmap-planning/SKILL.md"

func ContextPrompt(doc Document, focusedTask *Task, validation ValidationResult, skillBody string) string {
	return RoadmapContextPrompt(doc, focusedTask, validation, skillBody)
}

func RoadmapContextPrompt(doc Document, focusedTask *Task, validation ValidationResult, skillBody string) string {
	var b strings.Builder
	b.WriteString("You are in roadmapping mode. Treat the selected roadmap document as the primary workspace artifact.\n")
	b.WriteString("Do not mark roadmap tasks complete unless evidence is provided by tool results or tests.\n\n")
	fmt.Fprintf(&b, "Roadmap: %s\nPath: %s\nGoal: %s\n", doc.Title, doc.Path, doc.Goal)
	if focusedTask != nil {
		fmt.Fprintf(&b, "Focused task: %s (%s)\n", focusedTask.Text, focusedTask.ID)
	}
	if len(validation.Diagnostics) > 0 {
		b.WriteString("\nValidation diagnostics:\n")
		for _, diagnostic := range validation.Diagnostics {
			fmt.Fprintf(&b, "- %s:%d %s\n", diagnostic.Path, diagnostic.Line, diagnostic.Message)
		}
	}
	if strings.TrimSpace(skillBody) != "" {
		b.WriteString("\nRoadmap planning skill:\n")
		b.WriteString(strings.TrimSpace(skillBody))
		b.WriteString("\n")
	}
	return b.String()
}

func LoadPlanningSkillBody(workspace string) (string, error) {
	raw, err := os.ReadFile(filepath.Join(workspace, planningSkillPath))
	if os.IsNotExist(err) {
		return "", nil
	}
	if err != nil {
		return "", fmt.Errorf("read roadmap planning skill: %w", err)
	}
	return string(raw), nil
}
