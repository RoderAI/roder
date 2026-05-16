package skills

import (
	"path/filepath"
	"strings"

	godeskills "github.com/pandelisz/gode/internal/godex/skills"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

type State struct {
	Installed   []viewmodel.SettingsSkillItem
	Recommended []viewmodel.SettingsRecommendedSkillItem
	InstalledN  int
	EnabledN    int
}

func ViewState(installed []godeskills.ManagedSkill, recommended []godeskills.RecommendedSkillState) State {
	state := State{
		Installed:   make([]viewmodel.SettingsSkillItem, 0, len(installed)),
		Recommended: make([]viewmodel.SettingsRecommendedSkillItem, 0, len(recommended)),
	}
	nameCounts := map[string]int{}
	for _, skill := range installed {
		if strings.TrimSpace(skill.Name) != "" {
			nameCounts[skill.Name]++
		}
	}
	for _, skill := range installed {
		item := viewmodel.SettingsSkillItem{
			Name:             skill.Name,
			DisplayName:      interfaceDisplayName(skill),
			Description:      firstNonEmpty(interfaceShortDescription(skill), skill.Description),
			ShortDescription: interfaceShortDescription(skill),
			Path:             skill.Path,
			Source:           skill.Source,
			Scope:            firstNonEmpty(string(skill.Scope), skillScope(skill.Path)),
			State:            string(skill.State),
			DependencyHints:  dependencyHints(skill.Dependencies),
			Diagnostic:       skill.Diagnostic,
			AmbiguousName:    skill.Name != "" && nameCounts[skill.Name] > 1,
			Enabled:          skill.State == godeskills.ActivationEnabled,
		}
		if skill.Name != "" {
			state.InstalledN++
		}
		if item.Enabled {
			state.EnabledN++
		}
		state.Installed = append(state.Installed, item)
	}
	for _, skill := range recommended {
		state.Recommended = append(state.Recommended, viewmodel.SettingsRecommendedSkillItem{
			Name:   skill.Name,
			Source: skill.Source,
			State:  string(skill.State),
		})
	}
	return state
}

func interfaceDisplayName(skill godeskills.ManagedSkill) string {
	if skill.Interface == nil {
		return ""
	}
	return strings.TrimSpace(skill.Interface.DisplayName)
}

func interfaceShortDescription(skill godeskills.ManagedSkill) string {
	if skill.Interface == nil {
		return ""
	}
	return strings.TrimSpace(skill.Interface.ShortDescription)
}

func dependencyHints(deps *godeskills.SkillDependencies) []string {
	if deps == nil {
		return nil
	}
	hints := make([]string, 0, len(deps.Tools))
	for _, tool := range deps.Tools {
		parts := []string{}
		if strings.TrimSpace(tool.Type) != "" {
			parts = append(parts, tool.Type)
		}
		if strings.TrimSpace(tool.Value) != "" {
			parts = append(parts, tool.Value)
		}
		if strings.TrimSpace(tool.Transport) != "" {
			parts = append(parts, tool.Transport)
		}
		if len(parts) == 0 {
			continue
		}
		hints = append(hints, strings.Join(parts, ":"))
	}
	return hints
}

func firstNonEmpty(values ...string) string {
	for _, value := range values {
		if strings.TrimSpace(value) != "" {
			return value
		}
	}
	return ""
}

func SelectedSkill(items []viewmodel.SettingsSkillItem, index int) (viewmodel.SettingsSkillItem, bool) {
	if len(items) == 0 {
		return viewmodel.SettingsSkillItem{}, false
	}
	if index < 0 {
		index = 0
	}
	if index >= len(items) {
		index = len(items) - 1
	}
	return items[index], true
}

func MissingRecommendedNames(items []viewmodel.SettingsRecommendedSkillItem) []string {
	names := make([]string, 0, len(items))
	for _, item := range items {
		if item.State == string(godeskills.ActivationMissing) {
			names = append(names, item.Name)
		}
	}
	return names
}

func skillScope(path string) string {
	if path == "" {
		return ""
	}
	clean := filepath.ToSlash(path)
	switch {
	case strings.Contains(clean, "/.agents/skills/"):
		return "project"
	case strings.Contains(clean, "/.gode/skills/"):
		return "global"
	default:
		return "local"
	}
}
