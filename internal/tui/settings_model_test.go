package tui

import (
	"context"
	"path/filepath"
	"strings"
	"testing"
	"time"

	tea "charm.land/bubbletea/v2"
	"github.com/pandelisz/gode/internal/godex"
	"github.com/pandelisz/gode/internal/godex/codexauth"
	"github.com/pandelisz/gode/internal/tui/dialogs"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

func TestSettingsMenuCodexSignInRunsAuth(t *testing.T) {
	dataDir := t.TempDir()
	app, err := godex.New(context.Background(), godex.Config{
		DataDir:     dataDir,
		Workspace:   filepath.Join(t.TempDir(), "workspace"),
		Provider:    "mock",
		AutoApprove: true,
	})
	if err != nil {
		t.Fatalf("app: %v", err)
	}
	defer app.Close(context.Background())

	model := New(app)
	model.codexLogin = func(_ context.Context, gotDataDir string) (codexauth.Tokens, string, error) {
		if gotDataDir != dataDir {
			t.Fatalf("data dir = %q, want %q", gotDataDir, dataDir)
		}
		return codexauth.Tokens{AccountID: "acct_test"}, "", nil
	}
	model.openSettings()
	model.settings.MenuIndex = 2

	updated, cmd := model.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	got := updated.(Model)
	if got.settings.Open {
		t.Fatal("settings should close while codex sign-in runs")
	}
	if got.status != "opening browser for codex sign-in" {
		t.Fatalf("status = %q", got.status)
	}
	if cmd == nil {
		t.Fatal("expected codex sign-in command")
	}

	updated, _ = got.Update(cmd())
	got = updated.(Model)
	if got.status != "signed in to codex: acct_test" {
		t.Fatalf("status = %q", got.status)
	}
}

func TestHeaderShowsCodexProviderWhenCodexSubscriptionIsActive(t *testing.T) {
	dataDir := t.TempDir()
	if err := (codexauth.Store{DataDir: dataDir}).Save(codexauth.Tokens{Refresh: "refresh"}); err != nil {
		t.Fatalf("save codex auth: %v", err)
	}
	app, err := godex.New(context.Background(), godex.Config{
		DataDir:     dataDir,
		Workspace:   filepath.Join(t.TempDir(), "workspace"),
		Provider:    godex.ProviderOpenAI,
		Model:       godex.DefaultModelID,
		Reasoning:   godex.ReasoningMedium,
		AutoApprove: true,
	})
	if err != nil {
		t.Fatalf("app: %v", err)
	}
	defer app.Close(context.Background())

	model := New(app)
	view := model.View().Content
	if !strings.Contains(view, "codex/"+godex.DefaultModelID) {
		t.Fatalf("header should show codex subscription model label:\n%s", view)
	}
	if strings.Contains(view, "openai/"+godex.DefaultModelID) {
		t.Fatalf("header should not show openai provider while codex auth is active:\n%s", view)
	}
}

func TestSettingsMenuEnterOpensModelList(t *testing.T) {
	model := New(nil)
	model.openSettings()
	updated, _ := model.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	got := updated.(Model)

	if got.settings.Screen != dialogs.ScreenModels {
		t.Fatalf("settings screen = %v, want models", got.settings.Screen)
	}
	view := got.View().Content
	for _, want := range []string{"Models", "GPT-5.5", "GPT-5.4-Mini"} {
		if !strings.Contains(view, want) {
			t.Fatalf("view should render model list %q:\n%s", want, view)
		}
	}
}

func TestSettingsMenuClickOpensModelList(t *testing.T) {
	model := New(nil)
	model.width = 80
	model.height = 24
	model.openSettings()
	_ = model.View()

	click := clickZone(t, model, viewmodel.SettingsMenuItemZoneID("models"))
	updated, _ := model.Update(click)
	got := updated.(Model)

	if got.settings.Screen != dialogs.ScreenModels {
		t.Fatalf("settings screen = %v, want models", got.settings.Screen)
	}
}

func TestSettingsFastModeToggleSavesAndUpdatesApp(t *testing.T) {
	dataDir := t.TempDir()
	app, err := godex.New(context.Background(), godex.Config{
		DataDir:     dataDir,
		Workspace:   filepath.Join(t.TempDir(), "workspace"),
		Provider:    "mock",
		Model:       godex.DefaultModelID,
		Reasoning:   godex.ReasoningMedium,
		AutoApprove: true,
	})
	if err != nil {
		t.Fatalf("app: %v", err)
	}
	defer app.Close(context.Background())

	model := New(app)
	model.openSettings()
	model.settings.MenuIndex = 1
	updated, _ := model.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	got := updated.(Model)

	if !got.settings.Open {
		t.Fatal("settings dialog should remain open after toggling fast mode")
	}
	if !app.Config.FastMode {
		t.Fatal("app fast mode = false")
	}
	settings, err := godex.LoadSettings(dataDir)
	if err != nil {
		t.Fatalf("load settings: %v", err)
	}
	if !settings.FastMode {
		t.Fatal("saved fast mode = false")
	}
	view := got.View().Content
	for _, want := range []string{"Fast Mode", "on"} {
		if !strings.Contains(view, want) {
			t.Fatalf("view should render %q after fast mode toggle:\n%s", want, view)
		}
	}
}

func TestSettingsModelSelectionSavesDefaultAndUpdatesApp(t *testing.T) {
	dataDir := t.TempDir()
	app, err := godex.New(context.Background(), godex.Config{
		DataDir:     dataDir,
		Workspace:   filepath.Join(t.TempDir(), "workspace"),
		Provider:    godex.ProviderOpenAI,
		Model:       godex.DefaultModelID,
		AutoApprove: true,
	})
	if err != nil {
		t.Fatalf("app: %v", err)
	}
	defer app.Close(context.Background())

	model := New(app)
	model.openSettings()
	updated, _ := model.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	updated, _ = updated.Update(tea.KeyPressMsg{Code: tea.KeyDown})
	updated, _ = updated.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	got := updated.(Model)

	if got.settings.Screen != dialogs.ScreenReasoning {
		t.Fatalf("settings screen = %v, want reasoning", got.settings.Screen)
	}
	view := got.View().Content
	for _, want := range []string{"Reasoning", "Low", "Medium", "High", "XHigh"} {
		if !strings.Contains(view, want) {
			t.Fatalf("view should render reasoning option %q:\n%s", want, view)
		}
	}

	updated, _ = got.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	got = updated.(Model)

	wantModel := godex.BuiltInModels(false)[1].ID
	wantReasoning := godex.ReasoningMedium
	if got.settings.Open {
		t.Fatal("settings dialog should close after choosing reasoning")
	}
	if app.Config.Model != wantModel {
		t.Fatalf("app model = %q, want %q", app.Config.Model, wantModel)
	}
	if app.Config.Reasoning != wantReasoning {
		t.Fatalf("app reasoning = %q, want %q", app.Config.Reasoning, wantReasoning)
	}
	settings, err := godex.LoadSettings(dataDir)
	if err != nil {
		t.Fatalf("load settings: %v", err)
	}
	if settings.DefaultModel != wantModel {
		t.Fatalf("saved default model = %q, want %q", settings.DefaultModel, wantModel)
	}
	if settings.DefaultReasoning != wantReasoning {
		t.Fatalf("saved default reasoning = %q, want %q", settings.DefaultReasoning, wantReasoning)
	}
	if !got.input.Focused() {
		t.Fatal("composer should refocus after saving settings")
	}
}

func TestSettingsModelSelectionPersistsBeforeReasoningConfirmation(t *testing.T) {
	dataDir := t.TempDir()
	app, err := godex.New(context.Background(), godex.Config{
		DataDir:     dataDir,
		Workspace:   filepath.Join(t.TempDir(), "workspace"),
		Provider:    "mock",
		Model:       godex.DefaultModelID,
		Reasoning:   godex.ReasoningMedium,
		AutoApprove: true,
	})
	if err != nil {
		t.Fatalf("app: %v", err)
	}
	defer app.Close(context.Background())

	model := New(app)
	model.openSettings()
	updated, _ := model.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	updated, _ = updated.Update(tea.KeyPressMsg{Code: tea.KeyDown})
	updated, _ = updated.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	got := updated.(Model)

	wantModel := godex.BuiltInModels(false)[1].ID
	if got.settings.Screen != dialogs.ScreenReasoning {
		t.Fatalf("settings screen = %v, want reasoning", got.settings.Screen)
	}
	if app.Config.Model != wantModel {
		t.Fatalf("app model = %q, want %q", app.Config.Model, wantModel)
	}
	settings, err := godex.LoadSettings(dataDir)
	if err != nil {
		t.Fatalf("load settings: %v", err)
	}
	if settings.DefaultModel != wantModel {
		t.Fatalf("saved default model = %q, want %q", settings.DefaultModel, wantModel)
	}
}

func TestSettingsModelClickOpensReasoningAndReasoningClickSavesDefault(t *testing.T) {
	dataDir := t.TempDir()
	app, err := godex.New(context.Background(), godex.Config{
		DataDir:     dataDir,
		Workspace:   filepath.Join(t.TempDir(), "workspace"),
		Provider:    godex.ProviderOpenAI,
		Model:       godex.DefaultModelID,
		AutoApprove: true,
	})
	if err != nil {
		t.Fatalf("app: %v", err)
	}
	defer app.Close(context.Background())

	model := New(app)
	model.width = 80
	model.height = 24
	model.openSettings()
	model.settings.OpenModels()
	_ = model.View()

	wantModel := godex.BuiltInModels(false)[1].ID
	click := clickZone(t, model, viewmodel.SettingsModelZoneID(wantModel))
	updated, _ := model.Update(click)
	got := updated.(Model)

	if got.settings.Screen != dialogs.ScreenReasoning {
		t.Fatalf("settings screen = %v, want reasoning", got.settings.Screen)
	}
	_ = got.View()
	click = clickZone(t, got, viewmodel.SettingsReasoningZoneID(godex.ReasoningHigh))
	updated, _ = got.Update(click)
	got = updated.(Model)

	if got.settings.Open {
		t.Fatal("settings dialog should close after clicking reasoning")
	}
	if app.Config.Model != wantModel {
		t.Fatalf("app model = %q, want %q", app.Config.Model, wantModel)
	}
	if app.Config.Reasoning != godex.ReasoningHigh {
		t.Fatalf("app reasoning = %q, want %q", app.Config.Reasoning, godex.ReasoningHigh)
	}
	settings, err := godex.LoadSettings(dataDir)
	if err != nil {
		t.Fatalf("load settings: %v", err)
	}
	if settings.DefaultModel != wantModel {
		t.Fatalf("saved default model = %q, want %q", settings.DefaultModel, wantModel)
	}
	if settings.DefaultReasoning != godex.ReasoningHigh {
		t.Fatalf("saved default reasoning = %q, want %q", settings.DefaultReasoning, godex.ReasoningHigh)
	}
}

func TestSettingsReasoningListUsesSelectedModelOptions(t *testing.T) {
	model := New(nil)
	model.openSettings()
	model.settings.Models = []godex.ModelConfig{
		{
			ID:               "custom-model",
			DisplayName:      "Custom Model",
			Provider:         godex.ProviderOpenAI,
			DefaultReasoning: godex.ReasoningHigh,
			SupportedReasoning: []godex.ReasoningOption{
				{Effort: godex.ReasoningLow, Description: "low only for this model"},
				{Effort: godex.ReasoningHigh, Description: "high only for this model"},
			},
		},
	}
	model.settings.OpenModels()

	updated, _ := model.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	got := updated.(Model)

	if got.settings.Screen != dialogs.ScreenReasoning {
		t.Fatalf("settings screen = %v, want reasoning", got.settings.Screen)
	}
	view := got.View().Content
	for _, want := range []string{"Low", "High"} {
		if !strings.Contains(view, want) {
			t.Fatalf("view should render selected model reasoning %q:\n%s", want, view)
		}
	}
	for _, notWant := range []string{"Medium", "XHigh"} {
		if strings.Contains(view, notWant) {
			t.Fatalf("view should not render unsupported reasoning %q:\n%s", notWant, view)
		}
	}
}

func TestSettingsSubscreenEscapeReturnsToMenu(t *testing.T) {
	model := New(nil)
	model.openSettings()
	updated, _ := model.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	updated, _ = updated.Update(tea.KeyPressMsg{Code: tea.KeyEscape})
	got := updated.(Model)

	if !got.settings.Open {
		t.Fatal("settings dialog should stay open")
	}
	if got.settings.Screen != dialogs.ScreenMenu {
		t.Fatalf("settings screen = %v, want menu", got.settings.Screen)
	}
}

func TestSettingsDialogSavesModelAndUpdatesApp(t *testing.T) {
	dataDir := t.TempDir()
	app, err := godex.New(context.Background(), godex.Config{
		DataDir:     dataDir,
		Workspace:   filepath.Join(t.TempDir(), "workspace"),
		Provider:    "mock",
		Model:       "gpt-old",
		AutoApprove: true,
	})
	if err != nil {
		t.Fatalf("app: %v", err)
	}
	defer app.Close(context.Background())

	model := New(app)
	model.openSettings()
	model.settings.OpenModels()
	model.settings.ModelIndex = 1
	updated, _ := model.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	updated, _ = updated.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	got := updated.(Model)

	wantModel := godex.BuiltInModels(false)[1].ID
	if got.settings.Open {
		t.Fatal("settings dialog should close after save")
	}
	if app.Config.Model != wantModel {
		t.Fatalf("app model = %q, want %q", app.Config.Model, wantModel)
	}
	settings, err := godex.LoadSettings(dataDir)
	if err != nil {
		t.Fatalf("load settings: %v", err)
	}
	if settings.DefaultModel != wantModel {
		t.Fatalf("saved default model = %q, want %q", settings.DefaultModel, wantModel)
	}
	if !got.input.Focused() {
		t.Fatal("composer should refocus after saving settings")
	}
}

func clickZone(t *testing.T, model Model, id string) tea.MouseClickMsg {
	t.Helper()
	deadline := time.Now().Add(100 * time.Millisecond)
	zone := model.zones.Get(id)
	for (zone == nil || zone.IsZero()) && time.Now().Before(deadline) {
		time.Sleep(time.Millisecond)
		zone = model.zones.Get(id)
	}
	if zone == nil || zone.IsZero() {
		t.Fatalf("zone %q missing", id)
	}
	return tea.MouseClickMsg{X: zone.StartX, Y: zone.StartY, Button: tea.MouseLeft}
}

func keyCtrlP() tea.KeyPressMsg {
	return tea.KeyPressMsg{Code: 'p', Mod: tea.ModCtrl}
}
