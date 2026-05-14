package skills

import (
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"strings"
)

var validName = regexp.MustCompile(`^[a-zA-Z0-9]+(-[a-zA-Z0-9]+)*$`)

type SkillScope string

const (
	SkillScopeRepo   SkillScope = "repo"
	SkillScopeUser   SkillScope = "user"
	SkillScopeSystem SkillScope = "system"
	SkillScopeAdmin  SkillScope = "admin"
)

type Skill struct {
	Name             string
	Description      string
	ShortDescription string
	Compatibility    []string
	License          string
	Metadata         map[string]string
	Interface        *SkillInterface
	Dependencies     *SkillDependencies
	Policy           *SkillPolicy
	Path             string
	Scope            SkillScope
	PluginID         string
	Body             string
	Content          string
	Root             string
}

type Diagnostic struct {
	Path    string
	Message string
}

type SkillInterface struct {
	DisplayName      string `json:"display_name,omitempty" toml:"display_name,omitempty" yaml:"display_name,omitempty"`
	ShortDescription string `json:"short_description,omitempty" toml:"short_description,omitempty" yaml:"short_description,omitempty"`
	IconSmall        string `json:"icon_small,omitempty" toml:"icon_small,omitempty" yaml:"icon_small,omitempty"`
	IconLarge        string `json:"icon_large,omitempty" toml:"icon_large,omitempty" yaml:"icon_large,omitempty"`
	BrandColor       string `json:"brand_color,omitempty" toml:"brand_color,omitempty" yaml:"brand_color,omitempty"`
	DefaultPrompt    string `json:"default_prompt,omitempty" toml:"default_prompt,omitempty" yaml:"default_prompt,omitempty"`
}

type SkillDependencies struct {
	Tools []SkillToolDependency `json:"tools,omitempty" toml:"tools,omitempty" yaml:"tools,omitempty"`
}

type SkillToolDependency struct {
	Type        string `json:"type,omitempty" toml:"type,omitempty" yaml:"type,omitempty"`
	Value       string `json:"value,omitempty" toml:"value,omitempty" yaml:"value,omitempty"`
	Description string `json:"description,omitempty" toml:"description,omitempty" yaml:"description,omitempty"`
	Transport   string `json:"transport,omitempty" toml:"transport,omitempty" yaml:"transport,omitempty"`
	Command     string `json:"command,omitempty" toml:"command,omitempty" yaml:"command,omitempty"`
	URL         string `json:"url,omitempty" toml:"url,omitempty" yaml:"url,omitempty"`
}

type SkillPolicy struct {
	AllowImplicitInvocation *bool    `json:"allow_implicit_invocation,omitempty" toml:"allow_implicit_invocation,omitempty" yaml:"allow_implicit_invocation,omitempty"`
	Products                []string `json:"products,omitempty" toml:"products,omitempty" yaml:"products,omitempty"`
}

func ParseFile(path string) (Skill, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return Skill{}, fmt.Errorf("read skill %s: %w", path, err)
	}
	skill, err := ParseWithPath(string(data), path)
	if err != nil {
		return Skill{}, fmt.Errorf("parse skill %s: %w", path, err)
	}
	return skill, nil
}

func Parse(text string) (Skill, error) {
	return ParseWithPath(text, "")
}

func ParseWithPath(text string, path string) (Skill, error) {
	text = strings.ReplaceAll(text, "\r\n", "\n")
	frontmatter, body, err := splitFrontmatter(text)
	if err != nil {
		return Skill{}, err
	}
	skill, err := parseSkillFrontmatter(frontmatter)
	if err != nil {
		return Skill{}, err
	}
	skill.Path = path
	if skill.Name == "" && path != "" {
		skill.Name = filepath.Base(filepath.Dir(path))
	}
	if skill.Name == "" {
		return Skill{}, fmt.Errorf("missing field `name`")
	}
	skill.Name = cleanSingleLine(skill.Name, maxNameLen)
	if !validName.MatchString(skill.Name) {
		return Skill{}, fmt.Errorf("invalid skill name %q", skill.Name)
	}
	skill.Description = cleanSingleLine(skill.Description, maxDescriptionLen)
	if skill.Description == "" {
		return Skill{}, fmt.Errorf("missing field `description`")
	}
	skill.ShortDescription = cleanSingleLine(skill.ShortDescription, maxShortDescriptionLen)
	skill.License = cleanSingleLine(skill.License, maxNameLen)
	skill.Body = strings.TrimSpace(body)
	skill.Content = text
	if path != "" {
		loadOpenAIMetadata(filepath.Dir(path), &skill)
	}
	return skill, nil
}
