package completions

import (
	"sort"
	"strings"

	"github.com/pandelisz/gode/internal/godex/skills"
)

type SkillItem struct {
	Name        string
	Description string
}

func Skills(catalog []skills.Skill, query string, limit int) []SkillItem {
	query = strings.TrimPrefix(strings.TrimSpace(strings.ToLower(query)), "$")
	if limit <= 0 {
		limit = 50
	}
	var items []SkillItem
	for _, skill := range catalog {
		name := strings.TrimSpace(skill.Name)
		if name == "" {
			continue
		}
		if query != "" && !strings.Contains(strings.ToLower(name), query) && !strings.Contains(strings.ToLower(skill.Description), query) {
			continue
		}
		items = append(items, SkillItem{Name: name, Description: skill.Description})
	}
	sort.Slice(items, func(i, j int) bool { return items[i].Name < items[j].Name })
	if len(items) > limit {
		return items[:limit]
	}
	return items
}
