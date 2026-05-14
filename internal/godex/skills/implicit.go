package skills

import (
	"path/filepath"
	"strings"
)

func DetectImplicitInvocation(catalog Catalog, config Config, command string, workdir string) (Skill, bool) {
	disabled := DisabledSkillPaths(catalog.Skills, config)
	tokens := shellFields(command)
	if len(tokens) == 0 {
		return Skill{}, false
	}
	if script := scriptRunToken(tokens); script != "" {
		if skill, ok := skillForScriptPath(catalog.Skills, disabled, script, workdir); ok {
			return skill, true
		}
	}
	if commandReadsFile(tokens) {
		for _, token := range tokens[1:] {
			if strings.HasPrefix(token, "-") {
				continue
			}
			path := resolveCommandPath(token, workdir)
			for _, skill := range catalog.Skills {
				if _, ok := disabled[skillIdentity(skill)]; ok {
					continue
				}
				if canonicalPath(skill.Path) == path && allowsImplicit(skill) {
					return skill, true
				}
			}
		}
	}
	return Skill{}, false
}

func skillForScriptPath(skills []Skill, disabled map[string]struct{}, script string, workdir string) (Skill, bool) {
	path := resolveCommandPath(script, workdir)
	for _, skill := range skills {
		if _, ok := disabled[skillIdentity(skill)]; ok || !allowsImplicit(skill) {
			continue
		}
		scriptsDir := canonicalPath(filepath.Join(filepath.Dir(skill.Path), "scripts"))
		if path == scriptsDir || strings.HasPrefix(path, scriptsDir+string(filepath.Separator)) {
			return skill, true
		}
	}
	return Skill{}, false
}

func allowsImplicit(skill Skill) bool {
	if skill.Policy == nil || skill.Policy.AllowImplicitInvocation == nil {
		return true
	}
	return *skill.Policy.AllowImplicitInvocation
}

func scriptRunToken(tokens []string) string {
	if len(tokens) < 2 {
		return ""
	}
	runner := strings.TrimSuffix(strings.ToLower(filepath.Base(tokens[0])), ".exe")
	switch runner {
	case "python", "python3", "bash", "zsh", "sh", "node", "deno", "ruby", "perl", "pwsh":
	default:
		return ""
	}
	for _, token := range tokens[1:] {
		if token == "--" || strings.HasPrefix(token, "-") {
			continue
		}
		lower := strings.ToLower(token)
		for _, ext := range []string{".py", ".sh", ".js", ".ts", ".rb", ".pl", ".ps1"} {
			if strings.HasSuffix(lower, ext) {
				return token
			}
		}
		return ""
	}
	return ""
}

func commandReadsFile(tokens []string) bool {
	if len(tokens) == 0 {
		return false
	}
	switch strings.ToLower(filepath.Base(tokens[0])) {
	case "cat", "sed", "head", "tail", "less", "more", "bat", "awk":
		return true
	default:
		return false
	}
}

func resolveCommandPath(path string, workdir string) string {
	path = strings.TrimSpace(path)
	path = strings.TrimPrefix(path, "skill://")
	if !filepath.IsAbs(path) {
		path = filepath.Join(workdir, path)
	}
	return canonicalPath(path)
}

func shellFields(command string) []string {
	var fields []string
	var b strings.Builder
	var quote rune
	escaped := false
	for _, r := range command {
		switch {
		case escaped:
			b.WriteRune(r)
			escaped = false
		case r == '\\':
			escaped = true
		case quote != 0:
			if r == quote {
				quote = 0
			} else {
				b.WriteRune(r)
			}
		case r == '\'' || r == '"':
			quote = r
		case r == ' ' || r == '\t' || r == '\n':
			if b.Len() > 0 {
				fields = append(fields, b.String())
				b.Reset()
			}
		default:
			b.WriteRune(r)
		}
	}
	if b.Len() > 0 {
		fields = append(fields, b.String())
	}
	return fields
}
