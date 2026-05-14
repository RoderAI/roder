package skills

import (
	"context"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	"github.com/pandelisz/gode/internal/godex/provider"
)

func TestDiscoverFindsSkillsInConfiguredOrder(t *testing.T) {
	root := t.TempDir()
	workspace := filepath.Join(root, "workspace")
	dataDir := filepath.Join(root, "data")
	homeDir := filepath.Join(root, "home")
	codexHome := filepath.Join(root, "codex")

	writeSkill(t, filepath.Join(workspace, ".agents", "skills", "go-tests"), "go-tests", "project go tests")
	writeSkill(t, filepath.Join(workspace, "pkg", "api", ".agents", "skills", "api-skill"), "api-skill", "api skill")
	writeSkill(t, filepath.Join(workspace, ".gode", "skills", "repo-skill"), "repo-skill", "repo skill")
	writeSkill(t, filepath.Join(dataDir, "skills", "data-skill"), "data-skill", "data skill")
	writeSkill(t, filepath.Join(homeDir, ".agents", "skills", "agents-global"), "agents-global", "agents global skill")
	writeSkill(t, filepath.Join(homeDir, ".codex", "skills", "codex-global"), "codex-global", "codex global skill")
	writeSkill(t, filepath.Join(homeDir, ".gode", "skills", "global-skill"), "global-skill", "global skill")
	writeSkill(t, filepath.Join(codexHome, "skills", "codex-skill"), "codex-skill", "codex skill")
	writeSkill(t, filepath.Join(dataDir, "skills", "go-tests"), "go-tests", "shadowed")

	catalog := Discover(DiscoverOptions{
		Workspace: workspace,
		DataDir:   dataDir,
		HomeDir:   homeDir,
		Env:       []string{"CODEX_HOME=" + codexHome},
	})

	if got := skillNames(catalog.Skills); !reflect.DeepEqual(got, []string{"api-skill", "go-tests", "repo-skill", "agents-global", "codex-global", "codex-skill", "data-skill", "global-skill", "go-tests"}) {
		t.Fatalf("skills = %#v diagnostics=%#v", got, catalog.Diagnostics)
	}
	if catalog.Skills[1].Description != "project go tests" || catalog.Skills[8].Description != "shadowed" {
		t.Fatalf("same-name skills were not preserved by path: %#v", catalog.Skills)
	}
}

func TestDiscoverReturnsDiagnosticsForInvalidSkills(t *testing.T) {
	workspace := t.TempDir()
	mustWrite(t, filepath.Join(workspace, ".agents", "skills", "bad", "SKILL.md"), `---
name: bad_name
description: bad
---
body
`)

	catalog := Discover(DiscoverOptions{Workspace: workspace, HomeDir: filepath.Join(workspace, "home")})
	if len(catalog.Skills) != 0 {
		t.Fatalf("skills = %#v", catalog.Skills)
	}
	if len(catalog.Diagnostics) != 1 || !strings.Contains(catalog.Diagnostics[0].Message, "invalid skill name") {
		t.Fatalf("diagnostics = %#v", catalog.Diagnostics)
	}
}

func TestApplyInvocationsInjectsFoundSkillsOnceAndLeavesUnknownText(t *testing.T) {
	catalog := Catalog{Skills: []Skill{
		{Name: "go-tests", Body: "Run Go tests."},
		{Name: "review", Body: "Review carefully."},
	}}

	result := ApplyInvocations("please $go-tests and $unknown then $go-tests plus $review", catalog)
	if result.Prompt != "please and $unknown then plus" {
		t.Fatalf("prompt = %q", result.Prompt)
	}
	if got := skillNames(result.Invoked); !reflect.DeepEqual(got, []string{"go-tests", "review"}) {
		t.Fatalf("invoked = %#v", got)
	}
	if len(result.Messages) != 2 {
		t.Fatalf("messages = %#v", result.Messages)
	}
	for _, msg := range result.Messages {
		if msg.Role != provider.RoleUser {
			t.Fatalf("message role = %q", msg.Role)
		}
	}
	if !strings.Contains(result.Messages[0].Content, `<name>go-tests</name>`) || strings.Count(result.Messages[0].Content, "Run Go tests.") != 1 {
		t.Fatalf("first skill message = %q", result.Messages[0].Content)
	}
	if !strings.Contains(result.Messages[1].Content, `<name>review</name>`) {
		t.Fatalf("second skill message = %q", result.Messages[1].Content)
	}
}

func TestApplyInvocationsLeavesPromptUnchangedWhenNoSkillFound(t *testing.T) {
	prompt := "keep\n  $unknown   spacing"
	result := ApplyInvocations(prompt, Catalog{Skills: []Skill{{Name: "known", Body: "body"}}})
	if result.Prompt != prompt {
		t.Fatalf("prompt = %q, want %q", result.Prompt, prompt)
	}
	if len(result.Messages) != 0 || len(result.Invoked) != 0 {
		t.Fatalf("result = %#v", result)
	}
}

func TestApplyInvocationsReportsDisabledSkill(t *testing.T) {
	catalog := Catalog{Skills: []Skill{{
		Name: "go-tests",
		Path: "/skills/go-tests/SKILL.md",
		Body: "Run Go tests.",
	}}}
	config := Config{Rules: []ConfigRule{{Name: "go-tests", Enabled: false}}}
	result := ApplyInvocationsWithConfig("use $go-tests and $unknown", catalog, config)
	result.Diagnostics = append(result.Diagnostics, DisabledMentionDiagnostics("use $go-tests and $unknown", catalog, config)...)

	if result.Prompt != "use $go-tests and $unknown" {
		t.Fatalf("prompt = %q", result.Prompt)
	}
	if len(result.Messages) != 0 || len(result.Invoked) != 0 {
		t.Fatalf("disabled skill should not inject: %#v", result)
	}
	if len(result.Diagnostics) != 1 || !strings.Contains(result.Diagnostics[0].Message, "disabled") || result.Diagnostics[0].Path == "" {
		t.Fatalf("diagnostics = %#v", result.Diagnostics)
	}
}

func TestParseFrontmatterFields(t *testing.T) {
	skill, err := Parse(`---
name: go-tests
description: Run tests
compatibility: [gode, codex]
license: MIT
metadata:
  owner: ml-labs
---
Run the suite.
`)
	if err != nil {
		t.Fatalf("parse: %v", err)
	}
	if skill.Name != "go-tests" || skill.Description != "Run tests" || skill.License != "MIT" {
		t.Fatalf("skill = %#v", skill)
	}
	if !reflect.DeepEqual(skill.Compatibility, []string{"gode", "codex"}) {
		t.Fatalf("compatibility = %#v", skill.Compatibility)
	}
	if skill.Metadata["owner"] != "ml-labs" {
		t.Fatalf("metadata = %#v", skill.Metadata)
	}
	if skill.Body != "Run the suite." || !strings.Contains(skill.Content, "description: Run tests") {
		t.Fatalf("body = %q", skill.Body)
	}
}

func TestInstallCopiesLocalSkillDirectory(t *testing.T) {
	root := t.TempDir()
	source := filepath.Join(root, "source", "go-tests")
	writeSkill(t, source, "go-tests", "Run tests")
	mustWrite(t, filepath.Join(source, "assets", "note.txt"), "asset")
	workspace := filepath.Join(root, "workspace")

	result, err := Install(context.Background(), InstallOptions{Source: source, Workspace: workspace, Project: true})
	if err != nil {
		t.Fatalf("install: %v", err)
	}
	target := filepath.Join(workspace, ".agents", "skills", "go-tests")
	if len(result.Installed) != 1 || result.Installed[0].Path != target {
		t.Fatalf("result = %#v", result)
	}
	data, err := os.ReadFile(filepath.Join(target, "assets", "note.txt"))
	if err != nil {
		t.Fatalf("read copied asset: %v", err)
	}
	if string(data) != "asset" {
		t.Fatalf("asset = %q", string(data))
	}
}

func TestInstallMultipleSkillsRequiresFilterOrYes(t *testing.T) {
	root := t.TempDir()
	source := filepath.Join(root, "source")
	writeSkill(t, filepath.Join(source, "go-tests"), "go-tests", "Run tests")
	writeSkill(t, filepath.Join(source, "review"), "review", "Review")
	workspace := filepath.Join(root, "workspace")

	if _, err := Install(context.Background(), InstallOptions{Source: source, Workspace: workspace, Project: true}); err == nil {
		t.Fatal("install should require --skill or --yes for multi-skill sources")
	}
	result, err := Install(context.Background(), InstallOptions{Source: source, Workspace: workspace, Project: true, SkillNames: []string{"review"}})
	if err != nil {
		t.Fatalf("install filtered: %v", err)
	}
	if len(result.Installed) != 1 || result.Installed[0].Name != "review" {
		t.Fatalf("filtered result = %#v", result)
	}
}

func TestInstallClonesGitSourceToTempDir(t *testing.T) {
	root := t.TempDir()
	tempClone := filepath.Join(root, "clone")
	dataDir := filepath.Join(root, "data")
	result, err := Install(context.Background(), InstallOptions{
		Source:  "https://example.com/repo.git",
		DataDir: dataDir,
		Global:  true,
		TempDir: tempClone,
		CloneRunner: func(_ context.Context, source string, target string) error {
			if source != "https://example.com/repo.git" || target != tempClone {
				t.Fatalf("clone source=%q target=%q", source, target)
			}
			writeSkill(t, filepath.Join(target, "go-tests"), "go-tests", "Run tests")
			return nil
		},
	})
	if err != nil {
		t.Fatalf("install git: %v", err)
	}
	target := filepath.Join(dataDir, "skills", "go-tests")
	if len(result.Installed) != 1 || result.Installed[0].Path != target {
		t.Fatalf("result = %#v", result)
	}
	if _, err := os.Stat(filepath.Join(target, "SKILL.md")); err != nil {
		t.Fatalf("installed skill: %v", err)
	}
}

func skillNames(skills []Skill) []string {
	names := make([]string, 0, len(skills))
	for _, skill := range skills {
		names = append(names, skill.Name)
	}
	return names
}

func writeSkill(t *testing.T, dir string, name string, description string) {
	t.Helper()
	mustWrite(t, filepath.Join(dir, "SKILL.md"), `---
name: `+name+`
description: `+description+`
compatibility: [gode]
license: MIT
metadata:
  fixture: true
---
Skill body for `+name+`.
`)
}

func mustWrite(t *testing.T, path string, data string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if err := os.WriteFile(path, []byte(data), 0o644); err != nil {
		t.Fatalf("write: %v", err)
	}
}
