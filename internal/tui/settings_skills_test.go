package tui

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"
	"github.com/pandelisz/gode/internal/godex"
	godeskills "github.com/pandelisz/gode/internal/godex/skills"
	"github.com/pandelisz/gode/internal/tui/dialogs"
)

func TestSettingsMenuShowsSkillsCounts(t *testing.T) {
	app := newSkillsTestApp(t)
	defer app.Close(context.Background())

	model := New(app)
	model.openSettings()
	view := model.View().Content
	for _, want := range []string{"Skills", "1/1 enabled"} {
		if !strings.Contains(view, want) {
			t.Fatalf("settings view missing %q:\n%s", want, view)
		}
	}
}

func TestSettingsEnterOpensSkillsScreen(t *testing.T) {
	app := newSkillsTestApp(t)
	defer app.Close(context.Background())

	model := New(app)
	model.openSettings()
	model.settings.MenuIndex = settingsMenuIndex(t, model.settings, "skills")

	updated, _ := model.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	got := updated.(Model)
	if got.settings.Screen != dialogs.ScreenSkills {
		t.Fatalf("screen = %v, want skills", got.settings.Screen)
	}
	view := got.View().Content
	for _, want := range []string{"Installed Skills", "go-development", "space toggle"} {
		if !strings.Contains(view, want) {
			t.Fatalf("skills view missing %q:\n%s", want, view)
		}
	}
}

func TestSettingsSpaceTogglesInstalledSkill(t *testing.T) {
	app := newSkillsTestApp(t)
	defer app.Close(context.Background())

	model := New(app)
	model.openSettings()
	model.settings.OpenSkills()

	updated, _ := model.Update(tea.KeyPressMsg{Code: ' ', Text: " "})
	got := updated.(Model)
	if got.settings.Skills[0].Enabled {
		t.Fatalf("skill should be disabled in dialog: %#v", got.settings.Skills[0])
	}
	settings, err := godex.LoadSettings(app.Config.DataDir)
	if err != nil {
		t.Fatalf("load settings: %v", err)
	}
	if settings.ActiveSkills["go-development"] {
		t.Fatalf("active skills = %#v", settings.ActiveSkills)
	}
}

func TestSettingsRecommendedScreenAndInstallAllCommand(t *testing.T) {
	app := newSkillsTestApp(t)
	defer app.Close(context.Background())
	var commands [][]string
	app.SkillManager.RunCommand = func(_ context.Context, command []string) (string, string, error) {
		commands = append(commands, command)
		return "ok", "", nil
	}

	model := New(app)
	model.openSettings()
	model.settings.OpenSkills()

	updated, _ := model.Update(tea.KeyPressMsg{Code: 'r', Text: "r"})
	got := updated.(Model)
	if got.settings.Screen != dialogs.ScreenSkillRecommendations {
		t.Fatalf("screen = %v, want recommended", got.settings.Screen)
	}
	view := got.View().Content
	if !strings.Contains(view, "Recommended Skills") || !strings.Contains(view, "pandelisz/gode@repo-navigation") {
		t.Fatalf("recommended view missing expected content:\n%s", view)
	}

	updated, cmd := got.Update(tea.KeyPressMsg{Code: 'a', Text: "a"})
	got = updated.(Model)
	if cmd == nil {
		t.Fatal("expected install-all command")
	}
	updated, _ = got.Update(cmd())
	got = updated.(Model)
	if len(commands) != len(godeskills.RecommendedDefaultSkills)-1 {
		t.Fatalf("commands = %#v", commands)
	}
	if !strings.Contains(got.status, "installed") {
		t.Fatalf("status = %q", got.status)
	}
	settings, err := godex.LoadSettings(app.Config.DataDir)
	if err != nil {
		t.Fatalf("load settings: %v", err)
	}
	if !settings.ActiveSkills["repo-navigation"] || settings.SkillSources["repo-navigation"] != "pandelisz/gode@repo-navigation" {
		t.Fatalf("settings = %#v", settings)
	}
}

func TestSavingModelSettingsPreservesActiveSkills(t *testing.T) {
	app := newSkillsTestApp(t)
	defer app.Close(context.Background())
	if err := app.SkillManager.SetEnabled(context.Background(), "go-development", false); err != nil {
		t.Fatalf("disable skill: %v", err)
	}

	model := New(app)
	model.openSettings()
	model.settings.MenuIndex = settingsMenuIndex(t, model.settings, "fast-mode")
	updated, _ := model.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	got := updated.(Model)
	if got.settings.Err != "" {
		t.Fatalf("settings error = %q", got.settings.Err)
	}
	settings, err := godex.LoadSettings(app.Config.DataDir)
	if err != nil {
		t.Fatalf("load settings: %v", err)
	}
	if settings.ActiveSkills["go-development"] {
		t.Fatalf("active skills were not preserved: %#v", settings.ActiveSkills)
	}
}

func newSkillsTestApp(t *testing.T) *godex.App {
	t.Helper()
	root := t.TempDir()
	writeTUISkill(t, filepath.Join(root, ".agents", "skills", "go-development"), "go-development", "Go development")
	app, err := godex.New(context.Background(), godex.Config{
		DataDir:     filepath.Join(t.TempDir(), "data"),
		Workspace:   root,
		Provider:    "mock",
		Model:       "mock",
		Reasoning:   "none",
		AutoApprove: true,
	})
	if err != nil {
		t.Fatalf("app: %v", err)
	}
	return app
}

func writeTUISkill(t *testing.T, dir string, name string, description string) {
	t.Helper()
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatalf("mkdir skill: %v", err)
	}
	body := "---\nname: " + name + "\ndescription: " + description + "\n---\nUse this skill.\n"
	if err := os.WriteFile(filepath.Join(dir, "SKILL.md"), []byte(body), 0o644); err != nil {
		t.Fatalf("write skill: %v", err)
	}
}

func settingsMenuIndex(t *testing.T, settings dialogs.Settings, id string) int {
	t.Helper()
	for i, item := range settings.MenuItems() {
		if item.ID == id {
			return i
		}
	}
	t.Fatalf("menu item %q not found in %#v", id, settings.MenuItems())
	return 0
}
