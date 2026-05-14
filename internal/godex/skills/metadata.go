package skills

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"gopkg.in/yaml.v3"
)

const (
	skillFileName               = "SKILL.md"
	maxNameLen                  = 64
	maxDescriptionLen           = 1024
	maxShortDescriptionLen      = maxDescriptionLen
	maxDefaultPromptLen         = maxDescriptionLen
	maxDependencyTypeLen        = maxNameLen
	maxDependencyTransportLen   = maxNameLen
	maxDependencyValueLen       = maxDescriptionLen
	maxDependencyDescriptionLen = maxDescriptionLen
	maxDependencyCommandLen     = maxDescriptionLen
	maxDependencyURLLen         = maxDescriptionLen
	defaultMaxScanDepth         = 6
	defaultMaxSkillDirsPerRoot  = 2000
	metadataDirName             = "agents"
	openAIMetadataFileName      = "openai.yaml"
	assetsDirName               = "assets"
)

type skillFrontmatter struct {
	Name          string                   `yaml:"name"`
	Description   string                   `yaml:"description"`
	Compatibility []string                 `yaml:"compatibility"`
	License       string                   `yaml:"license"`
	Metadata      skillFrontmatterMetadata `yaml:"metadata"`
}

type skillFrontmatterMetadata struct {
	ShortDescription string            `yaml:"short-description"`
	Extra            map[string]string `yaml:",inline"`
}

type openAIMetadata struct {
	Interface    *openAIInterface   `yaml:"interface"`
	Dependencies *SkillDependencies `yaml:"dependencies"`
	Policy       *SkillPolicy       `yaml:"policy"`
}

type openAIInterface struct {
	DisplayName      string `yaml:"display_name"`
	ShortDescription string `yaml:"short_description"`
	IconSmall        string `yaml:"icon_small"`
	IconLarge        string `yaml:"icon_large"`
	BrandColor       string `yaml:"brand_color"`
	DefaultPrompt    string `yaml:"default_prompt"`
}

func splitFrontmatter(text string) (string, string, error) {
	if !strings.HasPrefix(text, "---\n") {
		return "", "", fmt.Errorf("missing YAML frontmatter delimited by ---")
	}
	end := strings.Index(text[4:], "\n---")
	if end < 0 {
		return "", "", fmt.Errorf("missing YAML frontmatter terminator")
	}
	frontmatter := text[4 : 4+end]
	body := strings.TrimLeft(text[4+end+len("\n---"):], "\n")
	return frontmatter, body, nil
}

func parseSkillFrontmatter(frontmatter string) (Skill, error) {
	var parsed skillFrontmatter
	if err := yaml.Unmarshal([]byte(frontmatter), &parsed); err != nil {
		return Skill{}, fmt.Errorf("invalid YAML: %w", err)
	}
	metadata := map[string]string{}
	for key, value := range parsed.Metadata.Extra {
		if key == "short-description" {
			continue
		}
		if cleaned := cleanSingleLine(value, maxDescriptionLen); cleaned != "" {
			metadata[key] = cleaned
		}
	}
	return Skill{
		Name:             parsed.Name,
		Description:      parsed.Description,
		ShortDescription: parsed.Metadata.ShortDescription,
		Compatibility:    cleanStringSlice(parsed.Compatibility, maxNameLen),
		License:          parsed.License,
		Metadata:         metadata,
	}, nil
}

func loadOpenAIMetadata(skillDir string, skill *Skill) {
	data, err := os.ReadFile(filepath.Join(skillDir, metadataDirName, openAIMetadataFileName))
	if err != nil {
		return
	}
	var parsed openAIMetadata
	if err := yaml.Unmarshal(data, &parsed); err != nil {
		return
	}
	if parsed.Interface != nil {
		skill.Interface = &SkillInterface{
			DisplayName:      cleanSingleLine(parsed.Interface.DisplayName, maxNameLen),
			ShortDescription: cleanSingleLine(parsed.Interface.ShortDescription, maxShortDescriptionLen),
			IconSmall:        resolveIconPath(skillDir, parsed.Interface.IconSmall),
			IconLarge:        resolveIconPath(skillDir, parsed.Interface.IconLarge),
			BrandColor:       cleanSingleLine(parsed.Interface.BrandColor, maxNameLen),
			DefaultPrompt:    cleanSingleLine(parsed.Interface.DefaultPrompt, maxDefaultPromptLen),
		}
	}
	if parsed.Dependencies != nil {
		skill.Dependencies = cleanDependencies(parsed.Dependencies)
	}
	if parsed.Policy != nil {
		skill.Policy = &SkillPolicy{
			AllowImplicitInvocation: parsed.Policy.AllowImplicitInvocation,
			Products:                cleanStringSlice(parsed.Policy.Products, maxNameLen),
		}
	}
}

func resolveIconPath(skillDir string, raw string) string {
	raw = strings.TrimSpace(raw)
	if raw == "" || filepath.IsAbs(raw) {
		return ""
	}
	clean := filepath.Clean(raw)
	if clean == "." || strings.HasPrefix(clean, ".."+string(filepath.Separator)) || clean == ".." {
		return ""
	}
	if clean != assetsDirName && !strings.HasPrefix(clean, assetsDirName+string(filepath.Separator)) {
		return ""
	}
	return filepath.Join(skillDir, clean)
}

func cleanDependencies(deps *SkillDependencies) *SkillDependencies {
	if deps == nil || len(deps.Tools) == 0 {
		return nil
	}
	out := &SkillDependencies{Tools: make([]SkillToolDependency, 0, len(deps.Tools))}
	for _, tool := range deps.Tools {
		cleaned := SkillToolDependency{
			Type:        cleanSingleLine(tool.Type, maxDependencyTypeLen),
			Value:       cleanSingleLine(tool.Value, maxDependencyValueLen),
			Description: cleanSingleLine(tool.Description, maxDependencyDescriptionLen),
			Transport:   cleanSingleLine(tool.Transport, maxDependencyTransportLen),
			Command:     cleanSingleLine(tool.Command, maxDependencyCommandLen),
			URL:         cleanSingleLine(tool.URL, maxDependencyURLLen),
		}
		if cleaned.Type == "" && cleaned.Value == "" {
			continue
		}
		out.Tools = append(out.Tools, cleaned)
	}
	if len(out.Tools) == 0 {
		return nil
	}
	return out
}

func cleanStringSlice(values []string, limit int) []string {
	out := make([]string, 0, len(values))
	for _, value := range values {
		if cleaned := cleanSingleLine(value, limit); cleaned != "" {
			out = append(out, cleaned)
		}
	}
	return out
}

func cleanSingleLine(value string, limit int) string {
	value = strings.Join(strings.Fields(value), " ")
	if limit > 0 && len([]rune(value)) > limit {
		runes := []rune(value)
		value = string(runes[:limit])
	}
	return value
}
