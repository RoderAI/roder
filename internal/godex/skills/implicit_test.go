package skills

import (
	"path/filepath"
	"testing"
)

func TestDetectImplicitInvocationFromScriptAndSkillDoc(t *testing.T) {
	root := t.TempDir()
	skillDir := filepath.Join(root, "go-development")
	skillPath := filepath.Join(skillDir, "SKILL.md")
	scriptPath := filepath.Join(skillDir, "scripts", "check.sh")
	mustWrite(t, skillPath, "---\nname: go-development\ndescription: Go\n---\n")
	mustWrite(t, scriptPath, "#!/bin/sh\n")
	catalog := Catalog{Skills: []Skill{{Name: "go-development", Path: skillPath}}}

	if skill, ok := DetectImplicitInvocation(catalog, Config{}, "bash "+scriptPath, root); !ok || skill.Name != "go-development" {
		t.Fatalf("script implicit = %#v %v", skill, ok)
	}
	rel, _ := filepath.Rel(root, skillPath)
	if skill, ok := DetectImplicitInvocation(catalog, Config{}, "cat "+rel, root); !ok || skill.Name != "go-development" {
		t.Fatalf("doc implicit = %#v %v", skill, ok)
	}
}

func TestDetectImplicitInvocationRespectsPolicyAndConfig(t *testing.T) {
	root := t.TempDir()
	skillPath := filepath.Join(root, "no-implicit", "SKILL.md")
	scriptPath := filepath.Join(root, "no-implicit", "scripts", "run.sh")
	mustWrite(t, skillPath, "---\nname: no-implicit\ndescription: No implicit\n---\n")
	mustWrite(t, scriptPath, "#!/bin/sh\n")
	disabledImplicit := false
	catalog := Catalog{Skills: []Skill{{
		Name:   "no-implicit",
		Path:   skillPath,
		Policy: &SkillPolicy{AllowImplicitInvocation: &disabledImplicit},
	}}}
	if _, ok := DetectImplicitInvocation(catalog, Config{}, "bash "+scriptPath, root); ok {
		t.Fatal("policy should disable implicit invocation")
	}
	catalog.Skills[0].Policy = nil
	cfg := Config{Rules: []ConfigRule{{Name: "no-implicit", Enabled: false}}}
	if _, ok := DetectImplicitInvocation(catalog, cfg, "bash "+scriptPath, root); ok {
		t.Fatal("config should disable implicit invocation")
	}
}
