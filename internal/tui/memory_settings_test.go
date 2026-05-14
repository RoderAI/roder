package tui

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"
	"github.com/pandelisz/gode/internal/godex"
	"github.com/pandelisz/gode/internal/godex/memory"
	"github.com/pandelisz/gode/internal/tui/dialogs"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

func TestSettingsMenuShowsMemoriesValue(t *testing.T) {
	app := newMemorySettingsApp(t, memory.Config{
		Enabled:        false,
		AutoRecall:     true,
		AutoObserve:    false,
		EmbeddingModel: memory.DefaultEmbeddingModel,
		RecallLimit:    memory.DefaultRecallLimit,
	})

	model := newSizedMemorySettingsModel(app)
	model.openSettings()

	view := model.View().Content
	for _, want := range []string{"Memories", "off"} {
		if !strings.Contains(view, want) {
			t.Fatalf("settings view should render %q:\n%s", want, view)
		}
	}
}

func TestSettingsEnterOpensMemorySettingsScreen(t *testing.T) {
	app := newMemorySettingsApp(t, memory.Config{
		Enabled:        true,
		AutoRecall:     true,
		AutoObserve:    true,
		EmbeddingModel: memory.DefaultEmbeddingModel,
		RecallLimit:    7,
	})

	model := newSizedMemorySettingsModel(app)
	model.openSettings()
	model.settings.MenuIndex = settingsMenuIndex(t, model.settings, "memories")
	updated, _ := model.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	got := updated.(Model)

	if got.settings.Screen != dialogs.ScreenMemories {
		t.Fatalf("settings screen = %v, want memories", got.settings.Screen)
	}
	vm := got.settings.ViewModel()
	if vm == nil || vm.Screen != viewmodel.SettingsScreenMemories {
		t.Fatalf("viewmodel = %#v", vm)
	}
	for _, want := range []string{"enabled", "count", "database", "embedding-model", "auto-recall", "auto-observe", "recall-limit"} {
		if !hasMemoryRow(vm.Memory.Rows, want) {
			t.Fatalf("memory settings rows missing %q: %#v", want, vm.Memory.Rows)
		}
	}
	view := got.View().Content
	for _, want := range []string{
		"Memories",
		"Enabled",
		"Auto recall",
		"Auto observe",
		"Workspace memories",
		"Database",
		"Embedding model",
		memory.DefaultEmbeddingModel,
		"7",
	} {
		if !strings.Contains(view, want) {
			t.Fatalf("memory settings view should render %q:\n%s", want, view)
		}
	}
}

func TestSettingsSpaceTogglesMemoryEnablementAndPersists(t *testing.T) {
	dataDir := t.TempDir()
	app := newMemorySettingsAppWithDataDir(t, dataDir, memory.Config{
		Enabled:        false,
		AutoRecall:     true,
		AutoObserve:    false,
		EmbeddingModel: memory.DefaultEmbeddingModel,
		RecallLimit:    memory.DefaultRecallLimit,
	})

	model := newSizedMemorySettingsModel(app)
	model.openSettings()
	model.settings.OpenMemories()
	updated, _ := model.Update(tea.KeyPressMsg{Code: tea.KeySpace})
	got := updated.(Model)

	if !got.settings.Open {
		t.Fatal("settings dialog should remain open after toggling memories")
	}
	if !app.Config.Memories.Enabled || !appHasToolName(app, "memory_save") {
		t.Fatalf("memories not enabled: config=%#v specs=%#v", app.Config.Memories, app.Tools.Specs())
	}
	loaded, err := godex.LoadSettings(dataDir)
	if err != nil {
		t.Fatalf("load settings: %v", err)
	}
	if loaded.Memories.Enabled == nil || !*loaded.Memories.Enabled {
		t.Fatalf("persisted memories enabled = %#v", loaded.Memories.Enabled)
	}
	raw, err := os.ReadFile(filepath.Join(dataDir, "config.toml"))
	if err != nil {
		t.Fatalf("read config.toml: %v", err)
	}
	if !strings.Contains(string(raw), "[memories]") || !strings.Contains(string(raw), "enabled = true") {
		t.Fatalf("config.toml missing memories enablement:\n%s", string(raw))
	}
	view := got.View().Content
	if !strings.Contains(view, "Enabled") || !strings.Contains(view, "on") {
		t.Fatalf("memory settings view missing enabled state:\n%s", view)
	}
}

func TestSettingsEnterTogglesMemoryEnablementOnMemoryScreen(t *testing.T) {
	app := newMemorySettingsApp(t, memory.Config{
		Enabled:        true,
		AutoRecall:     true,
		AutoObserve:    false,
		EmbeddingModel: memory.DefaultEmbeddingModel,
		RecallLimit:    memory.DefaultRecallLimit,
	})

	model := newSizedMemorySettingsModel(app)
	model.openSettings()
	model.settings.OpenMemories()
	updated, _ := model.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	got := updated.(Model)

	if app.Config.Memories.Enabled || appHasToolName(app, "memory_save") {
		t.Fatalf("memories not disabled: config=%#v specs=%#v", app.Config.Memories, app.Tools.Specs())
	}
	if got.status != "memories off" {
		t.Fatalf("status = %q", got.status)
	}
}

func TestSettingsMemoryEnabledClickTogglesMemoryEnablement(t *testing.T) {
	app := newMemorySettingsApp(t, memory.Config{
		Enabled:        false,
		AutoRecall:     true,
		AutoObserve:    false,
		EmbeddingModel: memory.DefaultEmbeddingModel,
		RecallLimit:    memory.DefaultRecallLimit,
	})

	model := newSizedMemorySettingsModel(app)
	model.openSettings()
	model.settings.OpenMemories()
	_ = model.View()
	click := clickZone(t, model, viewmodel.SettingsMemoryZoneID("enabled"))

	updated, _ := model.Update(click)
	got := updated.(Model)

	if !app.Config.Memories.Enabled || !appHasToolName(app, "memory_save") {
		t.Fatalf("memories not enabled after click: config=%#v specs=%#v", app.Config.Memories, app.Tools.Specs())
	}
	if got.status != "memories on" {
		t.Fatalf("status = %q", got.status)
	}
}

func newMemorySettingsApp(t *testing.T, cfg memory.Config) *godex.App {
	t.Helper()
	return newMemorySettingsAppWithDataDir(t, t.TempDir(), cfg)
}

func newSizedMemorySettingsModel(app *godex.App) Model {
	model := New(app)
	model.width = 100
	model.height = 30
	return model
}

func newMemorySettingsAppWithDataDir(t *testing.T, dataDir string, memCfg memory.Config) *godex.App {
	t.Helper()
	app, err := godex.New(context.Background(), godex.Config{
		DataDir:     dataDir,
		Workspace:   filepath.Join(t.TempDir(), "workspace"),
		Provider:    "mock",
		AutoApprove: true,
		Memories:    memCfg,
	})
	if err != nil {
		t.Fatalf("app: %v", err)
	}
	t.Cleanup(func() { _ = app.Close(context.Background()) })
	return app
}

func appHasToolName(app *godex.App, name string) bool {
	for _, spec := range app.Tools.Specs() {
		if spec.Name == name {
			return true
		}
	}
	return false
}

func hasMemoryRow(rows []viewmodel.SettingsMemoryRow, id string) bool {
	for _, row := range rows {
		if row.ID == id {
			return true
		}
	}
	return false
}
