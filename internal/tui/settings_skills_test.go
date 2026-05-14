package tui

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"
	"github.com/pandelisz/gode/internal/godex"
	godeskills "github.com/pandelisz/gode/internal/godex/skills"
	"github.com/pandelisz/gode/internal/tui/dialogs"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
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
	if godeskills.IsSkillEnabled(settings.Skills, godeskills.Skill{Name: "go-development", Path: filepath.Join(app.Config.Workspace, ".agents", "skills", "go-development", "SKILL.md")}) {
		t.Fatalf("skills config = %#v", settings.Skills)
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
	if !godeskills.IsSkillEnabled(settings.Skills, godeskills.Skill{Name: "repo-navigation", Path: filepath.Join(app.Config.DataDir, "skills", "repo-navigation", "SKILL.md")}) || settings.SkillSources["repo-navigation"] != "pandelisz/gode@repo-navigation" {
		t.Fatalf("settings = %#v", settings)
	}
}

func TestSettingsInstallPromptOpensWithI(t *testing.T) {
	app := newSkillsTestApp(t)
	defer app.Close(context.Background())

	model := New(app)
	model.openSettings()
	model.settings.OpenSkills()

	updated, _ := model.Update(tea.KeyPressMsg{Code: 'i', Text: "i"})
	got := updated.(Model)
	if got.settings.Screen != dialogs.ScreenSkillInstall {
		t.Fatalf("screen = %v, want install", got.settings.Screen)
	}
	if !strings.Contains(got.View().Content, "Install Skill") {
		t.Fatalf("view missing install prompt:\n%s", got.View().Content)
	}
}

func TestSettingsInstallPromptRunsInstallAndRefreshesSkills(t *testing.T) {
	app := newSkillsTestApp(t)
	defer app.Close(context.Background())
	var commands [][]string
	app.SkillManager.RunCommand = func(_ context.Context, command []string) (string, string, error) {
		commands = append(commands, command)
		writeTUISkill(t, filepath.Join(app.Config.DataDir, "skills", "repo-navigation"), "repo-navigation", "Repo navigation")
		return "installed repo-navigation", "", nil
	}

	model := New(app)
	model.openSettings()
	model.settings.OpenSkills()
	model.settings.OpenSkillInstall()
	updated := typeSettingsText(t, model, "pandelisz/gode@repo-navigation")
	got := updated.(Model)
	updated, cmd := got.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	got = updated.(Model)
	if cmd == nil {
		t.Fatal("expected install command")
	}
	if !got.settings.InstallPrompt.Installing || !strings.Contains(got.status, "installing pandelisz/gode@repo-navigation") {
		t.Fatalf("install state = installing:%v status:%q", got.settings.InstallPrompt.Installing, got.status)
	}

	updated, _ = got.Update(cmd())
	got = updated.(Model)
	if len(commands) != 1 {
		t.Fatalf("commands = %#v", commands)
	}
	if got.settings.Screen != dialogs.ScreenSkills {
		t.Fatalf("screen = %v, want skills", got.settings.Screen)
	}
	if !hasSettingsSkill(got.settings.Skills, "repo-navigation", true) {
		t.Fatalf("skills = %#v", got.settings.Skills)
	}
	settings, err := godex.LoadSettings(app.Config.DataDir)
	if err != nil {
		t.Fatalf("load settings: %v", err)
	}
	if !godeskills.IsSkillEnabled(settings.Skills, godeskills.Skill{Name: "repo-navigation", Path: filepath.Join(app.Config.DataDir, "skills", "repo-navigation", "SKILL.md")}) {
		t.Fatalf("skills config = %#v", settings.Skills)
	}
	if len(got.messages) == 0 || !strings.Contains(got.messages[len(got.messages)-1].Body, "installed repo-navigation") {
		t.Fatalf("messages = %#v", got.messages)
	}
}

func TestSettingsInstallFailureShowsStderrAndKeepsFullTranscript(t *testing.T) {
	app := newSkillsTestApp(t)
	defer app.Close(context.Background())
	longStderr := strings.Repeat("installer failed with details ", 12)
	app.SkillManager.RunCommand = func(context.Context, []string) (string, string, error) {
		return "", longStderr, errors.New("exit status 1")
	}

	model := New(app)
	model.openSettings()
	model.settings.OpenSkills()
	model.settings.OpenSkillInstall()
	updated := typeSettingsText(t, model, "pandelisz/gode@terminal-debugging")
	got := updated.(Model)
	updated, cmd := got.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	got = updated.(Model)
	if cmd == nil {
		t.Fatal("expected install command")
	}
	updated, _ = got.Update(cmd())
	got = updated.(Model)
	if got.settings.Err == "" || len([]rune(got.settings.Err)) > 160 {
		t.Fatalf("dialog error = %q", got.settings.Err)
	}
	if hasSettingsSkill(got.settings.Skills, "terminal-debugging", true) {
		t.Fatalf("failed install should not add enabled skill: %#v", got.settings.Skills)
	}
	if len(got.messages) == 0 || !strings.Contains(got.messages[len(got.messages)-1].Body, strings.TrimSpace(longStderr)) {
		t.Fatalf("full stderr missing from transcript: %#v", got.messages)
	}
}

func TestSavingModelSettingsPreservesSkillsConfig(t *testing.T) {
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
	if godeskills.IsSkillEnabled(settings.Skills, godeskills.Skill{Name: "go-development", Path: filepath.Join(app.Config.Workspace, ".agents", "skills", "go-development", "SKILL.md")}) {
		t.Fatalf("skills config was not preserved: %#v", settings.Skills)
	}
}

func typeSettingsText(t *testing.T, model Model, text string) tea.Model {
	t.Helper()
	var updated tea.Model = model
	for _, r := range text {
		var cmd tea.Cmd
		updated, cmd = updated.Update(tea.KeyPressMsg{Code: r, Text: string(r)})
		if cmd != nil {
			t.Fatalf("unexpected command while typing %q", string(r))
		}
	}
	return updated
}

func hasSettingsSkill(items []viewmodel.SettingsSkillItem, name string, enabled bool) bool {
	for _, item := range items {
		if item.Name == name && item.Enabled == enabled {
			return true
		}
	}
	return false
}

func newSkillsTestApp(t *testing.T) *godex.App {
	t.Helper()
	root := t.TempDir()
	writeTUISkill(t, filepath.Join(root, ".agents", "skills", "go-development"), "go-development", "Go development")
	app, err := godex.New(context.Background(), godex.Config{
		DataDir:     filepath.Join(t.TempDir(), "data"),
		HomeDir:     filepath.Join(t.TempDir(), "home"),
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
