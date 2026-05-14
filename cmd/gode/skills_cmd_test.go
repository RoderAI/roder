package main

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/pandelisz/gode/internal/godex"
	godeskills "github.com/pandelisz/gode/internal/godex/skills"
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

func TestRunSkillsListJSONAndEnableDisable(t *testing.T) {
	root := t.TempDir()
	t.Setenv("HOME", filepath.Join(root, "home"))
	workspace := filepath.Join(root, "workspace")
	dataDir := filepath.Join(root, "data")
	writeSkillFixture(t, filepath.Join(workspace, ".agents", "skills", "go-tests"), "go-tests", "Run Go tests")

	disableOut := captureStdout(t, func() error {
		return runSkills(context.Background(), []string{"disable", "go-tests", "--workspace", workspace, "--data-dir", dataDir})
	})
	if !strings.Contains(disableOut, "disabled\tgo-tests") {
		t.Fatalf("disable output = %q", disableOut)
	}
	out := captureStdout(t, func() error {
		return runSkills(context.Background(), []string{"list", "--json", "--workspace", workspace, "--data-dir", dataDir})
	})
	var items []struct {
		Name  string `json:"Name"`
		State string `json:"State"`
	}
	if err := json.Unmarshal([]byte(out), &items); err != nil {
		t.Fatalf("json: %v\n%s", err, out)
	}
	if len(items) != 1 || items[0].Name != "go-tests" || items[0].State != "disabled" {
		t.Fatalf("items = %#v", items)
	}
	enableOut := captureStdout(t, func() error {
		return runSkills(context.Background(), []string{"enable", "go-tests", "--workspace", workspace, "--data-dir", dataDir})
	})
	if !strings.Contains(enableOut, "enabled\tgo-tests") {
		t.Fatalf("enable output = %q", enableOut)
	}
	settings, err := godex.LoadSettings(dataDir)
	if err != nil {
		t.Fatalf("load settings: %v", err)
	}
	if !godeskills.IsSkillEnabled(settings.Skills, godeskills.Skill{Name: "go-tests", Path: filepath.Join(workspace, ".agents", "skills", "go-tests", "SKILL.md")}) {
		t.Fatalf("skills config = %#v", settings.Skills)
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

func TestRunSkillsRecommendedJSONAndDryRun(t *testing.T) {
	root := t.TempDir()
	t.Setenv("HOME", filepath.Join(root, "home"))
	workspace := filepath.Join(root, "workspace")
	dataDir := filepath.Join(root, "data")

	recommended := captureStdout(t, func() error {
		return runSkills(context.Background(), []string{"recommended", "--json", "--workspace", workspace, "--data-dir", dataDir})
	})
	if !json.Valid([]byte(recommended)) || !strings.Contains(recommended, `"Name":"go-development"`) {
		t.Fatalf("recommended json:\n%s", recommended)
	}

	dryRun := captureStdout(t, func() error {
		return runSkills(context.Background(), []string{"install-recommended", "--all", "--dry-run", "--workspace", workspace, "--data-dir", dataDir})
	})
	for _, want := range []string{
		"npx --yes skills add pandelisz/gode@go-development --global",
		"npx --yes skills add pandelisz/gode@terminal-debugging --global",
	} {
		if !strings.Contains(dryRun, want) {
			t.Fatalf("dry-run missing %q:\n%s", want, dryRun)
		}
	}
}

func TestRunSkillsAddDryRunPrintsNPXCommand(t *testing.T) {
	root := t.TempDir()
	workspace := filepath.Join(root, "workspace with spaces")
	dataDir := filepath.Join(root, "data")
	out := captureStdout(t, func() error {
		return runSkills(context.Background(), []string{"add", "pandelisz/gode@go-development", "--project", "--dry-run", "--workspace", workspace, "--data-dir", dataDir})
	})
	if !strings.Contains(out, "npx --yes skills add pandelisz/gode@go-development --project --cwd '"+workspace+"'") {
		t.Fatalf("dry-run output = %q", out)
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
