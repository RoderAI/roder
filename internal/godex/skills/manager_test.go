package skills

import (
	"context"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

func TestManagerListSetEnabledAndRecommended(t *testing.T) {
	root := t.TempDir()
	writeSkill(t, root+"/.agents/skills/go-development", "go-development", "Go development")
	brokenPath := filepath.Join(root, ".agents", "skills", "broken", "SKILL.md")
	if err := os.MkdirAll(filepath.Dir(brokenPath), 0o755); err != nil {
		t.Fatalf("mkdir broken skill: %v", err)
	}
	if err := os.WriteFile(brokenPath, []byte("---\nname: bad name\n---\nbody"), 0o644); err != nil {
		t.Fatalf("write broken skill: %v", err)
	}
	settings := ActivationSettings{
		Skills:       Config{Rules: []ConfigRule{{Name: "go-development", Enabled: false}}},
		SkillSources: map[string]string{"go-development": "local"},
	}
	manager := &Manager{
		Workspace: root,
		LoadSettings: func(context.Context) (ActivationSettings, error) {
			return settings, nil
		},
		SaveSettings: func(_ context.Context, next ActivationSettings) error {
			settings = next
			return nil
		},
	}
	items, err := manager.List(context.Background())
	if err != nil {
		t.Fatalf("list: %v", err)
	}
	if len(items) != 2 || items[0].Name != "go-development" || items[0].State != ActivationDisabled || items[0].Source != "local" || items[0].Description != "Go development" || items[0].Path == "" {
		t.Fatalf("items = %#v", items)
	}
	if items[1].State != ActivationMissing || !strings.Contains(items[1].Diagnostic, "invalid skill name") || items[1].Path == "" {
		t.Fatalf("diagnostic item = %#v", items[1])
	}
	recommended, err := manager.Recommended(context.Background())
	if err != nil {
		t.Fatalf("recommended disabled: %v", err)
	}
	if recommended[0].Name != "go-development" || recommended[0].State != ActivationDisabled {
		t.Fatalf("recommended disabled = %#v", recommended)
	}
	if err := manager.SetEnabled(context.Background(), "go-development", true); err != nil {
		t.Fatalf("enable: %v", err)
	}
	if !IsSkillEnabled(settings.Skills, Skill{Name: "go-development", Path: filepath.Join(root, ".agents", "skills", "go-development", "SKILL.md")}) {
		t.Fatalf("settings = %#v", settings)
	}
	recommended, err = manager.Recommended(context.Background())
	if err != nil {
		t.Fatalf("recommended: %v", err)
	}
	if recommended[0].Name != "go-development" || recommended[0].State != ActivationEnabled {
		t.Fatalf("recommended = %#v", recommended)
	}
	if recommended[1].State != ActivationMissing {
		t.Fatalf("missing recommended = %#v", recommended[1])
	}
}

func TestManagerInstallLocalSourceRecordsSourceAndEnables(t *testing.T) {
	source := t.TempDir()
	writeSkill(t, source, "local-skill", "Local skill")
	dataDir := t.TempDir()
	settings := ActivationSettings{}
	manager := &Manager{
		DataDir: dataDir,
		LoadSettings: func(context.Context) (ActivationSettings, error) {
			return settings, nil
		},
		SaveSettings: func(_ context.Context, next ActivationSettings) error {
			settings = next
			return nil
		},
	}
	result, err := manager.Install(context.Background(), InstallRequest{Source: source, Scope: InstallScopeGlobal})
	if err != nil {
		t.Fatalf("install local: %v", err)
	}
	if result.Source != source {
		t.Fatalf("result = %#v", result)
	}
	if !IsSkillEnabled(settings.Skills, Skill{Name: "local-skill", Path: filepath.Join(dataDir, "skills", "local-skill", "SKILL.md")}) || settings.SkillSources["local-skill"] != source {
		t.Fatalf("settings = %#v", settings)
	}
	if _, err := os.Stat(filepath.Join(dataDir, "skills", "local-skill", "SKILL.md")); err != nil {
		t.Fatalf("installed skill missing: %v", err)
	}
}

func TestManagerInstallRecommendedUsesFakeRunner(t *testing.T) {
	var commands [][]string
	settings := ActivationSettings{}
	manager := &Manager{
		DataDir: "/data",
		LoadSettings: func(context.Context) (ActivationSettings, error) {
			return settings, nil
		},
		SaveSettings: func(_ context.Context, next ActivationSettings) error {
			settings = next
			return nil
		},
		RunCommand: func(_ context.Context, command []string) (string, string, error) {
			commands = append(commands, command)
			return "ok", "", nil
		},
	}
	results, err := manager.InstallRecommended(context.Background(), []string{"go-development"})
	if err != nil {
		t.Fatalf("install recommended: %v", err)
	}
	if len(results) != 1 || results[0].Stdout != "ok" {
		t.Fatalf("results = %#v", results)
	}
	want := []string{"npx", "--yes", "skills", "add", "pandelisz/gode@go-development", "--global"}
	if !reflect.DeepEqual(commands[0], want) {
		t.Fatalf("commands = %#v", commands)
	}
	if !IsSkillEnabled(settings.Skills, Skill{Name: "go-development", Path: filepath.Join("/data", "skills", "go-development", "SKILL.md")}) || settings.SkillSources["go-development"] != "pandelisz/gode@go-development" {
		t.Fatalf("settings = %#v", settings)
	}
}

func TestManagerInstallCapturesFailureOutput(t *testing.T) {
	manager := &Manager{
		RunCommand: func(context.Context, []string) (string, string, error) {
			return "out", "err", assertErr("failed")
		},
	}
	result, err := manager.Install(context.Background(), InstallRequest{Source: "pandelisz/gode@go-development"})
	if err == nil || !strings.Contains(err.Error(), "failed") {
		t.Fatalf("err = %v", err)
	}
	if result.Stdout != "out" || result.Stderr != "err" {
		t.Fatalf("result = %#v", result)
	}
}

type assertErr string

func (e assertErr) Error() string { return string(e) }
