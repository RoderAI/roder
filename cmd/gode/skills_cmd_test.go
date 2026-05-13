package main

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestRunSkillsListShowsScopeNameDescriptionAndPath(t *testing.T) {
	root := t.TempDir()
	t.Setenv("HOME", filepath.Join(root, "home"))
	workspace := filepath.Join(root, "workspace")
	dataDir := filepath.Join(root, "data")
	skillDir := filepath.Join(workspace, ".agents", "skills", "go-tests")
	writeSkillFixture(t, skillDir, "go-tests", "Run Go tests")

	output := captureStdout(t, func() error {
		return runSkills(context.Background(), []string{"list", "--workspace", workspace, "--data-dir", dataDir})
	})
	for _, want := range []string{"project", "go-tests", "Run Go tests", filepath.Join(skillDir, "SKILL.md")} {
		if !strings.Contains(output, want) {
			t.Fatalf("output missing %q:\n%s", want, output)
		}
	}
}

func TestRunSkillsAddInstallsProjectSkill(t *testing.T) {
	root := t.TempDir()
	t.Setenv("HOME", filepath.Join(root, "home"))
	source := filepath.Join(root, "source", "go-tests")
	workspace := filepath.Join(root, "workspace")
	dataDir := filepath.Join(root, "data")
	writeSkillFixture(t, source, "go-tests", "Run Go tests")

	output := captureStdout(t, func() error {
		return runSkills(context.Background(), []string{"add", source, "--project", "--workspace", workspace, "--data-dir", dataDir})
	})
	target := filepath.Join(workspace, ".agents", "skills", "go-tests")
	if !strings.Contains(output, "installed\tgo-tests\t"+target) {
		t.Fatalf("output = %q", output)
	}
	if _, err := os.Stat(filepath.Join(target, "SKILL.md")); err != nil {
		t.Fatalf("installed skill: %v", err)
	}
}

func writeSkillFixture(t *testing.T, dir string, name string, description string) {
	t.Helper()
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	data := `---
name: ` + name + `
description: ` + description + `
---
Skill body.
`
	if err := os.WriteFile(filepath.Join(dir, "SKILL.md"), []byte(data), 0o644); err != nil {
		t.Fatalf("write skill: %v", err)
	}
}
