package skills

import (
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"strings"
)

var validName = regexp.MustCompile(`^[a-zA-Z0-9]+(-[a-zA-Z0-9]+)*$`)

type Skill struct {
	Name          string
	Description   string
	Compatibility []string
	License       string
	Metadata      map[string]string
	Path          string
	Body          string
}

type Diagnostic struct {
	Path    string
	Message string
}

func ParseFile(path string) (Skill, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return Skill{}, fmt.Errorf("read skill %s: %w", path, err)
	}
	skill, err := Parse(string(data))
	if err != nil {
		return Skill{}, fmt.Errorf("parse skill %s: %w", path, err)
	}
	skill.Path = path
	if skill.Name == "" {
		skill.Name = filepath.Base(filepath.Dir(path))
	}
	if !validName.MatchString(skill.Name) {
		return Skill{}, fmt.Errorf("invalid skill name %q", skill.Name)
	}
	return skill, nil
}

func Parse(text string) (Skill, error) {
	text = strings.ReplaceAll(text, "\r\n", "\n")
	skill := Skill{Metadata: map[string]string{}}
	if !strings.HasPrefix(text, "---\n") {
		skill.Body = strings.TrimSpace(text)
		return skill, nil
	}
	end := strings.Index(text[4:], "\n---")
	if end < 0 {
		return Skill{}, fmt.Errorf("missing frontmatter terminator")
	}
	frontmatter := text[4 : 4+end]
	body := strings.TrimLeft(text[4+end+len("\n---"):], "\n")
	skill.Body = strings.TrimSpace(body)
	parseFrontmatter(&skill, frontmatter)
	return skill, nil
}

func parseFrontmatter(skill *Skill, frontmatter string) {
	section := ""
	for _, line := range strings.Split(frontmatter, "\n") {
		if strings.TrimSpace(line) == "" {
			continue
		}
		if strings.HasPrefix(line, " ") || strings.HasPrefix(line, "\t") {
			if section == "metadata" {
				key, value, ok := cutKeyValue(strings.TrimSpace(line))
				if ok {
					skill.Metadata[key] = cleanScalar(value)
				}
			}
			continue
		}
		key, value, ok := cutKeyValue(line)
		if !ok {
			continue
		}
		section = key
		switch key {
		case "name":
			skill.Name = cleanScalar(value)
		case "description":
			skill.Description = cleanScalar(value)
		case "compatibility":
			skill.Compatibility = parseStringList(value)
		case "license":
			skill.License = cleanScalar(value)
		case "metadata":
			if strings.TrimSpace(value) != "" && strings.TrimSpace(value) != "{}" {
				skill.Metadata[key] = cleanScalar(value)
			}
		}
	}
}

func cutKeyValue(line string) (string, string, bool) {
	key, value, ok := strings.Cut(line, ":")
	if !ok {
		return "", "", false
	}
	return strings.TrimSpace(key), strings.TrimSpace(value), true
}

func cleanScalar(value string) string {
	value = strings.TrimSpace(value)
	value = strings.Trim(value, `"'`)
	return value
}

func parseStringList(value string) []string {
	value = strings.TrimSpace(value)
	if value == "" {
		return nil
	}
	if strings.HasPrefix(value, "[") && strings.HasSuffix(value, "]") {
		value = strings.TrimSuffix(strings.TrimPrefix(value, "["), "]")
	}
	parts := strings.Split(value, ",")
	out := make([]string, 0, len(parts))
	for _, part := range parts {
		if cleaned := cleanScalar(part); cleaned != "" {
			out = append(out, cleaned)
		}
	}
	return out
}
