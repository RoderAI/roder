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
	for _, skill := range installed {
		item := viewmodel.SettingsSkillItem{
			Name:        skill.Name,
			Description: skill.Description,
			Path:        skill.Path,
			Source:      skill.Source,
			Scope:       skillScope(skill.Path),
			State:       string(skill.State),
			Diagnostic:  skill.Diagnostic,
			Enabled:     skill.State == godeskills.ActivationEnabled,
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
