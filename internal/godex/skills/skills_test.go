package skills

import (
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
	writeSkill(t, filepath.Join(homeDir, ".gode", "skills", "global-skill"), "global-skill", "global skill")
	writeSkill(t, filepath.Join(codexHome, "skills", "codex-skill"), "codex-skill", "codex skill")
	writeSkill(t, filepath.Join(dataDir, "skills", "go-tests"), "go-tests", "shadowed")

	catalog := Discover(DiscoverOptions{
		Workspace: workspace,
		DataDir:   dataDir,
		HomeDir:   homeDir,
		Env:       []string{"CODEX_HOME=" + codexHome},
	})

	if got := skillNames(catalog.Skills); !reflect.DeepEqual(got, []string{"go-tests", "api-skill", "repo-skill", "data-skill", "global-skill", "codex-skill"}) {
		t.Fatalf("skills = %#v diagnostics=%#v", got, catalog.Diagnostics)
	}
	if catalog.Skills[0].Description != "project go tests" {
		t.Fatalf("shadowing did not preserve first skill: %#v", catalog.Skills[0])
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
		if msg.Role != provider.RoleSystem {
			t.Fatalf("message role = %q", msg.Role)
		}
	}
	if !strings.Contains(result.Messages[0].Content, `<skill name="go-tests">`) || strings.Count(result.Messages[0].Content, "Run Go tests.") != 1 {
		t.Fatalf("first skill message = %q", result.Messages[0].Content)
	}
	if !strings.Contains(result.Messages[1].Content, `<skill name="review">`) {
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
	if skill.Body != "Run the suite." {
		t.Fatalf("body = %q", skill.Body)
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
