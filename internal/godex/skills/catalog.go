package skills

import "sort"

func IsSkillEnabled(activeSkills map[string]bool, skillName string) bool {
	if activeSkills == nil {
		return true
	}
	enabled, ok := activeSkills[skillName]
	if !ok {
		return true
	}
	return enabled
}

func SetSkillEnabled(activeSkills *map[string]bool, skillName string, enabled bool) {
	if *activeSkills == nil {
		*activeSkills = map[string]bool{}
	}
	(*activeSkills)[skillName] = enabled
}

func EnabledSkillNames(activeSkills map[string]bool, discovered []Skill) []string {
	names := make([]string, 0, len(discovered))
	for _, skill := range discovered {
		if IsSkillEnabled(activeSkills, skill.Name) {
			names = append(names, skill.Name)
		}
	}
	sort.Strings(names)
	return names
}
