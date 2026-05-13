package tui

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"
	"github.com/pandelisz/gode/internal/godex"
)

func TestAtOpensFileCompletionAndAttachesFile(t *testing.T) {
	workspace := t.TempDir()
	if err := os.WriteFile(filepath.Join(workspace, "README.md"), []byte("hello\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	app, err := godex.New(context.Background(), godex.Config{DataDir: t.TempDir(), Workspace: workspace, Provider: "mock", AutoApprove: true})
	if err != nil {
		t.Fatalf("app: %v", err)
	}
	defer app.Close(context.Background())

	model := New(app)
	defer model.cancelEvents()
	updated, _ := model.Update(tea.KeyPressMsg{Code: '@', Text: "@"})
	got := updated.(Model)
	if !got.completions.Open {
		t.Fatal("file completions should open")
	}

	updated, _ = got.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	got = updated.(Model)
	if got.input.Value() != "@README.md " {
		t.Fatalf("input = %q", got.input.Value())
	}
	if len(got.attachments) != 1 || got.attachments[0].Path != "README.md" {
		t.Fatalf("attachments = %#v", got.attachments)
	}
	if view := got.View().Content; !containsAll(view, "@README.md", "[text]") {
		t.Fatalf("attachment bar missing file:\n%s", view)
	}
}

func TestDollarOpensSkillCompletionAndInsertsSkill(t *testing.T) {
	workspace := t.TempDir()
	skillDir := filepath.Join(workspace, ".agents", "skills", "some-skill")
	if err := os.MkdirAll(skillDir, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(skillDir, "SKILL.md"), []byte("---\nname: some-skill\ndescription: does work\n---\nUse it.\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	app, err := godex.New(context.Background(), godex.Config{DataDir: t.TempDir(), Workspace: workspace, Provider: "mock", AutoApprove: true})
	if err != nil {
		t.Fatalf("app: %v", err)
	}
	defer app.Close(context.Background())

	model := New(app)
	defer model.cancelEvents()
	updated, _ := model.Update(tea.KeyPressMsg{Code: '$', Text: "$"})
	got := updated.(Model)
	if !got.completions.Open {
		t.Fatal("skill completions should open")
	}

	updated, _ = got.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	got = updated.(Model)
	if got.input.Value() != "$some-skill " {
		t.Fatalf("input = %q", got.input.Value())
	}
}

func TestTokenCompletionFiltersFileAndReplacesMention(t *testing.T) {
	workspace := t.TempDir()
	for name := range map[string]string{
		"README.md": "hello\n",
		"main.go":   "package main\n",
	} {
		if err := os.WriteFile(filepath.Join(workspace, name), []byte(name), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	app, err := godex.New(context.Background(), godex.Config{DataDir: t.TempDir(), Workspace: workspace, Provider: "mock", AutoApprove: true})
	if err != nil {
		t.Fatalf("app: %v", err)
	}
	defer app.Close(context.Background())

	model := New(app)
	defer model.cancelEvents()
	model.input.SetValue("read @REA")
	updated, _ := model.Update(tea.KeyPressMsg{Code: tea.KeyTab})
	got := updated.(Model)
	if !got.completions.Open || len(got.completions.Items) != 1 || got.completions.Items[0].ID != "README.md" {
		t.Fatalf("completions = %#v", got.completions)
	}

	updated, _ = got.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	got = updated.(Model)
	if got.input.Value() != "read @README.md " {
		t.Fatalf("input = %q", got.input.Value())
	}
	if len(got.attachments) != 1 || got.attachments[0].Path != "README.md" {
		t.Fatalf("attachments = %#v", got.attachments)
	}
}

func TestCompletionDialogAcceptsTypedSkillQuery(t *testing.T) {
	workspace := t.TempDir()
	for _, name := range []string{"api-skill", "docs-skill"} {
		skillDir := filepath.Join(workspace, ".agents", "skills", name)
		if err := os.MkdirAll(skillDir, 0o700); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(skillDir, "SKILL.md"), []byte("---\nname: "+name+"\ndescription: test\n---\n"), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	app, err := godex.New(context.Background(), godex.Config{DataDir: t.TempDir(), Workspace: workspace, Provider: "mock", AutoApprove: true})
	if err != nil {
		t.Fatalf("app: %v", err)
	}
	defer app.Close(context.Background())

	model := New(app)
	defer model.cancelEvents()
	updated, _ := model.Update(tea.KeyPressMsg{Code: '$', Text: "$"})
	got := updated.(Model)
	updated, _ = got.Update(tea.KeyPressMsg{Code: 'd', Text: "d"})
	got = updated.(Model)
	if len(got.completions.Items) != 1 || got.completions.Items[0].ID != "docs-skill" {
		t.Fatalf("completions = %#v", got.completions)
	}
	updated, _ = got.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	got = updated.(Model)
	if got.input.Value() != "$docs-skill " {
		t.Fatalf("input = %q", got.input.Value())
	}
}

func containsAll(text string, wants ...string) bool {
	for _, want := range wants {
		if !strings.Contains(text, want) {
			return false
		}
	}
	return true
}
