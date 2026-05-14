package tui

import (
	"context"
	"path/filepath"
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"
	"github.com/pandelisz/gode/internal/godex"
	"github.com/pandelisz/gode/internal/godex/provider"
	"github.com/pandelisz/gode/internal/tui/dialogs"
)

func TestSettingsCustomModelListShowsDisplayNameAndProvider(t *testing.T) {
	cfg := customModelConfig(t.TempDir(), t.TempDir())
	settings := dialogs.NewSettings(cfg)
	settings.OpenModels()
	vm := settings.ViewModel()
	if vm == nil {
		t.Fatal("missing view model")
	}
	found := false
	for _, item := range vm.Models {
		if item.ID == "local-deepseek" {
			found = true
			if item.DisplayName != "DeepSeek Local" || item.Provider != "deepseek" {
				t.Fatalf("custom model item = %#v", item)
			}
		}
	}
	if !found {
		t.Fatalf("custom model missing: %#v", vm.Models)
	}

	model := New(nil)
	model.width = 100
	model.height = 24
	model.settings = settings
	view := model.View().Content
	for _, want := range []string{"DeepSeek Local", "deepseek/local-deepseek"} {
		if !strings.Contains(view, want) {
			t.Fatalf("view missing %q:\n%s", want, view)
		}
	}
}

func TestSettingsCustomModelSelectionPersistsLocalModelID(t *testing.T) {
	dataDir := t.TempDir()
	app, err := godex.New(context.Background(), customModelConfig(dataDir, filepath.Join(t.TempDir(), "workspace")))
	if err != nil {
		t.Fatalf("app: %v", err)
	}
	defer app.Close(context.Background())

	model := New(app)
	model.openSettings()
	model.settings.OpenModels()
	found := false
	for i, item := range model.settings.Models {
		if item.ID == "local-deepseek" {
			model.settings.ModelIndex = i
			found = true
			break
		}
	}
	if !found {
		t.Fatalf("custom model missing: %#v", model.settings.Models)
	}
	updated, _ := model.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	got := updated.(Model)

	if app.Config.Model != "local-deepseek" {
		t.Fatalf("app model = %q", app.Config.Model)
	}
	if app.Config.Provider != "deepseek" {
		t.Fatalf("app provider = %q", app.Config.Provider)
	}
	if got.settings.Screen != dialogs.ScreenReasoning {
		t.Fatalf("settings screen = %v", got.settings.Screen)
	}
	settings, err := godex.LoadSettings(dataDir)
	if err != nil {
		t.Fatalf("load settings: %v", err)
	}
	if settings.DefaultModel != "local-deepseek" {
		t.Fatalf("saved default model = %q", settings.DefaultModel)
	}
}

func customModelConfig(dataDir string, workspace string) godex.Config {
	return godex.Config{
		DataDir:     dataDir,
		Workspace:   workspace,
		Provider:    "mock",
		Model:       "mock",
		Reasoning:   godex.ReasoningNone,
		AutoApprove: true,
		UserModels: map[string]provider.UserModelConfig{
			"local-deepseek": {
				Type:             string(provider.APITypeChatCompletions),
				Provider:         "deepseek",
				Model:            "deepseek-chat",
				DisplayName:      "DeepSeek Local",
				BaseURL:          "http://127.0.0.1:9/v1",
				DefaultReasoning: godex.ReasoningNone,
				ReasoningEfforts: []string{godex.ReasoningNone},
			},
		},
	}
}
