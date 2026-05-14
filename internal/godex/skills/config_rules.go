package skills

import (
	"path/filepath"
	"strings"
)

type Config struct {
	Rules []ConfigRule `json:"config,omitempty" toml:"config,omitempty"`
}

type ConfigRule struct {
	Name    string `json:"name,omitempty" toml:"name,omitempty"`
	Path    string `json:"path,omitempty" toml:"path,omitempty"`
	Enabled bool   `json:"enabled" toml:"enabled"`
}

type ConfigRuleSelectorKind string

const (
	ConfigRuleSelectorName ConfigRuleSelectorKind = "name"
	ConfigRuleSelectorPath ConfigRuleSelectorKind = "path"
)

type ResolvedConfigRule struct {
	SelectorKind ConfigRuleSelectorKind
	Name         string
	Path         string
	Enabled      bool
}

func ResolveConfigRules(config Config) []ResolvedConfigRule {
	out := make([]ResolvedConfigRule, 0, len(config.Rules))
	for _, rule := range config.Rules {
		name := strings.TrimSpace(rule.Name)
		path := strings.TrimSpace(rule.Path)
		if name == "" && path == "" {
			continue
		}
		if name != "" && path != "" {
			continue
		}
		if name != "" {
			out = append(out, ResolvedConfigRule{SelectorKind: ConfigRuleSelectorName, Name: name, Enabled: rule.Enabled})
			continue
		}
		out = append(out, ResolvedConfigRule{SelectorKind: ConfigRuleSelectorPath, Path: canonicalPath(path), Enabled: rule.Enabled})
	}
	return out
}

func DisabledSkillPaths(skills []Skill, config Config) map[string]struct{} {
	disabled := map[string]struct{}{}
	for _, rule := range ResolveConfigRules(config) {
		switch rule.SelectorKind {
		case ConfigRuleSelectorName:
			for _, skill := range skills {
				if skill.Name != rule.Name {
					continue
				}
				if rule.Enabled {
					delete(disabled, skillIdentity(skill))
				} else {
					disabled[skillIdentity(skill)] = struct{}{}
				}
			}
		case ConfigRuleSelectorPath:
			if rule.Enabled {
				delete(disabled, rule.Path)
			} else {
				disabled[rule.Path] = struct{}{}
			}
		}
	}
	return disabled
}

func IsSkillEnabled(config Config, skill Skill) bool {
	_, disabled := DisabledSkillPaths([]Skill{skill}, config)[skillIdentity(skill)]
	return !disabled
}

func SetSkillEnabled(config *Config, skill Skill, enabled bool) {
	if config == nil {
		return
	}
	path := canonicalPath(skill.Path)
	if path == "" {
		path = skillIdentity(skill)
	}
	next := config.Rules[:0]
	for _, rule := range config.Rules {
		if strings.TrimSpace(rule.Path) != "" && canonicalPath(rule.Path) == path {
			continue
		}
		next = append(next, rule)
	}
	config.Rules = append(next, ConfigRule{Path: path, Enabled: enabled})
}

func EnabledSkillNames(config Config, discovered []Skill) []string {
	disabled := DisabledSkillPaths(discovered, config)
	names := make([]string, 0, len(discovered))
	for _, skill := range discovered {
		if _, ok := disabled[skillIdentity(skill)]; !ok {
			names = append(names, skill.Name)
		}
	}
	return sortedStrings(names)
}

func canonicalPath(path string) string {
	path = strings.TrimSpace(path)
	if path == "" {
		return ""
	}
	if abs, err := filepath.Abs(path); err == nil {
		path = abs
	}
	if resolved, err := filepath.EvalSymlinks(path); err == nil {
		path = resolved
	}
	return filepath.Clean(path)
}

func skillIdentity(skill Skill) string {
	if path := canonicalPath(skill.Path); path != "" {
		return path
	}
	return "name:" + skill.Name
}

func SkillIdentity(skill Skill) string {
	return skillIdentity(skill)
}
