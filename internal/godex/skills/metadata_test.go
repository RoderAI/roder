package skills

import (
	"path/filepath"
	"testing"
)

func TestParseSkillMetadataAndOpenAIYaml(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "go-development")
	mustWrite(t, filepath.Join(dir, "SKILL.md"), `---
description:   Build   Go    code
metadata:
  short-description: Go dev
---
Use gofmt and go test.
`)
	mustWrite(t, filepath.Join(dir, "agents", "openai.yaml"), `
interface:
  display_name: Go Development
  short_description: Go tools
  icon_small: assets/icon-small.png
  icon_large: ../bad.png
  brand_color: "#00add8"
  default_prompt: Run the Go test suite
dependencies:
  tools:
    - type: cli
      value: go
      description: Go toolchain
      command: go test ./...
policy:
  allow_implicit_invocation: false
  products: [codex]
`)

	skill, err := ParseFile(filepath.Join(dir, "SKILL.md"))
	if err != nil {
		t.Fatalf("parse: %v", err)
	}
	if skill.Name != "go-development" || skill.Description != "Build Go code" || skill.ShortDescription != "Go dev" {
		t.Fatalf("skill = %#v", skill)
	}
	if skill.Interface == nil || skill.Interface.DisplayName != "Go Development" || skill.Interface.IconSmall != filepath.Join(dir, "assets", "icon-small.png") || skill.Interface.IconLarge != "" {
		t.Fatalf("interface = %#v", skill.Interface)
	}
	if skill.Dependencies == nil || len(skill.Dependencies.Tools) != 1 || skill.Dependencies.Tools[0].Value != "go" {
		t.Fatalf("dependencies = %#v", skill.Dependencies)
	}
	if skill.Policy == nil || skill.Policy.AllowImplicitInvocation == nil || *skill.Policy.AllowImplicitInvocation {
		t.Fatalf("policy = %#v", skill.Policy)
	}
	if skill.Content == "" || skill.Body != "Use gofmt and go test." {
		t.Fatalf("content/body = %#v / %q", skill.Content, skill.Body)
	}
}

func TestParseRequiresFrontmatterDescriptionAndValidName(t *testing.T) {
	if _, err := Parse("no frontmatter"); err == nil {
		t.Fatal("missing frontmatter should fail")
	}
	if _, err := Parse("---\nname: bad_name\ndescription: bad\n---\n"); err == nil {
		t.Fatal("invalid name should fail")
	}
	if _, err := Parse("---\nname: ok-name\n---\n"); err == nil {
		t.Fatal("missing description should fail")
	}
}
