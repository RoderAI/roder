package skills

import (
	"fmt"
	"path/filepath"
	"strings"
)

const (
	SkillsInstructionsOpenTag  = "<skills_instructions>"
	SkillsInstructionsCloseTag = "</skills_instructions>"
	defaultSkillMetadataBudget = 8000
	skillBudgetPercent         = 2
)

type RenderOptions struct {
	ContextWindow int
	Config        Config
}

type RenderReport struct {
	TotalCount                int
	IncludedCount             int
	OmittedCount              int
	TruncatedDescriptionCount int
	TruncatedDescriptionChars int
	Warning                   string
}

type RenderedAvailableSkills struct {
	Body   string
	Report RenderReport
}

func RenderAvailable(catalog Catalog, opts RenderOptions) RenderedAvailableSkills {
	disabled := DisabledSkillPaths(catalog.Skills, opts.Config)
	var enabled []Skill
	for _, skill := range catalog.Skills {
		if _, ok := disabled[skillIdentity(skill)]; !ok {
			enabled = append(enabled, skill)
		}
	}
	report := RenderReport{TotalCount: len(enabled)}
	if len(enabled) == 0 {
		return RenderedAvailableSkills{Report: report}
	}
	budget := defaultSkillMetadataBudget
	if opts.ContextWindow > 0 {
		budget = max(1, opts.ContextWindow*skillBudgetPercent/100*4)
	}
	aliases := skillRootAliases(catalog.Roots)
	lines := make([]string, 0, len(enabled))
	used := 0
	for _, skill := range enabled {
		path := skill.Path
		if alias := aliases.aliasFor(path); alias != "" {
			path = alias
		}
		desc := skill.Description
		line := skillLine(skill, path, desc)
		if used+len(line) > budget {
			remaining := budget - used - len(skillLine(skill, path, ""))
			if remaining > 20 {
				shortened := truncateRunes(desc, remaining)
				report.TruncatedDescriptionCount++
				report.TruncatedDescriptionChars += len([]rune(desc)) - len([]rune(shortened))
				line = skillLine(skill, path, shortened)
			} else {
				report.OmittedCount++
				continue
			}
		}
		used += len(line)
		lines = append(lines, line)
	}
	report.IncludedCount = len(lines)
	if report.OmittedCount > 0 {
		report.Warning = fmt.Sprintf("Exceeded skills context budget; %d additional skills were not included in the model-visible skills list.", report.OmittedCount)
	} else if report.TruncatedDescriptionCount > 0 && report.TruncatedDescriptionChars > 100 {
		report.Warning = "Skill descriptions were shortened to fit the skills context budget. Gode can still see every skill, but some descriptions are shorter."
	}
	body := renderAvailableSkillsBody(aliases.lines, lines)
	return RenderedAvailableSkills{Body: body, Report: report}
}

func renderAvailableSkillsBody(rootLines []string, skillLines []string) string {
	var lines []string
	lines = append(lines, "## Skills")
	if len(rootLines) == 0 {
		lines = append(lines, "A skill is a set of local instructions to follow that is stored in a `SKILL.md` file. Below is the list of skills that can be used. Each entry includes a name, description, and file path so you can open the source for full instructions when using a specific skill.")
	} else {
		lines = append(lines,
			"A skill is a set of local instructions to follow that is stored in a `SKILL.md` file. Below is the list of skills that can be used. Each entry includes a name, description, and a short path that can be expanded into an absolute path using the skill roots table.",
			"### Skill roots",
		)
		lines = append(lines, rootLines...)
	}
	lines = append(lines, "### Available skills")
	lines = append(lines, skillLines...)
	lines = append(lines,
		"### How to use skills",
		"- Discovery: The list above is the skills available in this session (name + description + path). Skill bodies live on disk at the listed paths.",
		"- Trigger rules: If the user names a skill (with `$SkillName` or plain text) OR the task clearly matches a skill's description shown above, you must use that skill for that turn. Multiple mentions mean use them all. Do not carry skills across turns unless re-mentioned.",
		"- Missing/blocked: If a named skill isn't in the list or the path can't be read, say so briefly and continue with the best fallback.",
		"- How to use a skill (progressive disclosure):",
		"  1) Open its `SKILL.md`; read only enough to follow the workflow.",
		"  2) Resolve relative paths from the directory containing that `SKILL.md`.",
		"  3) Load only the referenced files needed for the request.",
		"  4) If `scripts/` exist, prefer running or patching them instead of retyping large code blocks.",
		"  5) If `assets/` or templates exist, reuse them instead of recreating from scratch.",
		"- Coordination and sequencing: choose the minimal set of skills that covers the request and announce which skill(s) you are using.",
		"- Context hygiene: keep context small and avoid deep reference-chasing.",
	)
	return "\n" + strings.Join(lines, "\n") + "\n"
}

func skillLine(skill Skill, path string, description string) string {
	return fmt.Sprintf("- %s: %s (file: %s)", skill.Name, description, path)
}

type rootAliases struct {
	values map[string]string
	lines  []string
}

func skillRootAliases(roots []Root) rootAliases {
	out := rootAliases{values: map[string]string{}}
	for i, root := range roots {
		clean := canonicalPath(root.Path)
		if clean == "" {
			continue
		}
		alias := fmt.Sprintf("r%d", i)
		out.values[clean] = alias
		out.lines = append(out.lines, fmt.Sprintf("- `%s` = `%s`", alias, clean))
	}
	return out
}

func (a rootAliases) aliasFor(path string) string {
	path = canonicalPath(path)
	var bestRoot string
	var bestAlias string
	for root, alias := range a.values {
		if path == root || strings.HasPrefix(path, root+string(filepath.Separator)) {
			if len(root) > len(bestRoot) {
				bestRoot = root
				bestAlias = alias
			}
		}
	}
	if bestAlias == "" {
		return ""
	}
	rel, err := filepath.Rel(bestRoot, path)
	if err != nil || rel == "." {
		return bestAlias
	}
	return bestAlias + "/" + filepath.ToSlash(rel)
}

func truncateRunes(value string, maxRunes int) string {
	if maxRunes <= 0 {
		return ""
	}
	runes := []rune(value)
	if len(runes) <= maxRunes {
		return value
	}
	if maxRunes <= 3 {
		return string(runes[:maxRunes])
	}
	return string(runes[:maxRunes-3]) + "..."
}
