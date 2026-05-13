package tui

import (
	"context"
	"errors"
	"path/filepath"
	"strings"
	"testing"
	"time"

	tea "charm.land/bubbletea/v2"
	"github.com/pandelisz/gode/internal/godex"
	"github.com/pandelisz/gode/internal/godex/codexauth"
	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/tui/eventadapter"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

func TestModelAppendsAssistantDeltaEvents(t *testing.T) {
	app, err := godex.New(context.Background(), godex.Config{DataDir: t.TempDir(), Workspace: t.TempDir(), Provider: "mock", AutoApprove: true})
	if err != nil {
		t.Fatalf("app: %v", err)
	}
	defer app.Close(context.Background())

	model := New(app)
	updated, _ := model.Update(eventMsg{Event: eventbus.Event{Kind: eventbus.KindAssistantDelta, Payload: map[string]any{"text": "hello"}}})
	got := updated.(Model)
	if len(got.messages) != 1 || got.messages[0].Body != "hello" {
		t.Fatalf("messages = %#v", got.messages)
	}
}

func TestModelCoalescesAssistantDeltaEvents(t *testing.T) {
	model := New(nil)
	updated, _ := model.Update(eventMsg{Event: eventbus.Event{Kind: eventbus.KindAssistantDelta, Payload: map[string]any{"text": "hel"}}})
	updated, _ = updated.Update(eventMsg{Event: eventbus.Event{Kind: eventbus.KindAssistantDelta, Payload: map[string]any{"text": "lo"}}})

	got := updated.(Model)
	if len(got.messages) != 1 || got.messages[0].Body != "hello" {
		t.Fatalf("messages = %#v", got.messages)
	}
}

func TestModelDrainsBufferedFastEvents(t *testing.T) {
	app, err := godex.New(context.Background(), godex.Config{DataDir: t.TempDir(), Workspace: t.TempDir(), Provider: "mock", AutoApprove: true})
	if err != nil {
		t.Fatalf("app: %v", err)
	}
	defer app.Close(context.Background())

	model := New(app)
	defer model.cancelEvents()
	app.Bus.Publish(context.Background(), eventbus.Event{Kind: eventbus.KindAssistantDelta, Payload: map[string]any{"text": "hel"}})
	app.Bus.Publish(context.Background(), eventbus.Event{Kind: eventbus.KindAssistantDelta, Payload: map[string]any{"text": "lo"}})

	firstCmd := model.Init()
	if firstCmd == nil {
		t.Fatal("expected initial event wait command")
	}
	updated, nextCmd := model.Update(firstCmd())
	got := updated.(Model)
	if nextCmd == nil {
		t.Fatal("expected follow-up event wait command")
	}
	updated, _ = got.Update(nextCmd())
	got = updated.(Model)

	if len(got.messages) != 1 || got.messages[0].Body != "hello" {
		t.Fatalf("messages = %#v", got.messages)
	}
}

func TestModelSummarizesReadFileToolOutput(t *testing.T) {
	model := New(nil)
	fullContents := strings.Repeat("package main\n", 200)
	updated, _ := model.Update(eventMsg{Event: eventbus.Event{
		Kind: eventbus.KindToolCompleted,
		Payload: map[string]any{
			"tool": "read_file",
			"input": map[string]any{
				"path": "internal/godex/tools/registry.go",
			},
			"text": fullContents,
		},
	}})

	got := updated.(Model)
	if len(got.messages) != 1 {
		t.Fatalf("messages = %#v", got.messages)
	}
	if got.messages[0].Body != "read internal/godex/tools/registry.go" {
		t.Fatalf("tool body = %q", got.messages[0].Body)
	}
	if strings.Contains(got.messages[0].Body, "package main") {
		t.Fatalf("read_file timeline should not include file contents:\n%s", got.messages[0].Body)
	}
}

func TestEventAdapterStateEventsDoNotRenderEmptyTranscriptRows(t *testing.T) {
	model := New(nil)
	events := []eventbus.Event{
		{Kind: eventbus.KindPermissionResponded, Payload: map[string]any{"decision": "deny"}},
		{Kind: eventbus.KindMCPStateChanged, Payload: map[string]any{"server": "github", "state": "connected"}},
		{Kind: eventbus.KindLSPStateChanged, Payload: map[string]any{"server": "gopls", "state": "connected"}},
		{Kind: eventadapter.KindHookResult, Payload: map[string]any{"hook": "policy", "decision": "allow"}},
		{Kind: eventadapter.KindSessionUpdate, Payload: map[string]any{"title": "feature"}},
		{Kind: eventadapter.KindModelChanged, Payload: map[string]any{"model": "gpt-5.5"}},
	}

	var updated tea.Model = model
	for _, ev := range events {
		updated, _ = updated.Update(eventMsg{Event: ev})
	}
	got := updated.(Model)
	if len(got.messages) != 0 {
		t.Fatalf("state events should not render transcript rows: %#v", got.messages)
	}
	if strings.TrimSpace(got.status) == "" {
		t.Fatal("state events should leave a useful status")
	}
}

func TestModelShowsReasoningSummaryAboveComposer(t *testing.T) {
	model := New(nil)
	model.width = 80
	model.height = 24
	updated, _ := model.Update(eventMsg{Event: eventbus.Event{Kind: eventbus.KindReasoningSummaryDelta, Payload: map[string]any{"text": "Checking workspace"}}})
	updated, _ = updated.Update(eventMsg{Event: eventbus.Event{Kind: eventbus.KindReasoningSummaryDelta, Payload: map[string]any{"text": " before editing."}}})

	got := updated.(Model)
	if got.reasoningSummary != "Checking workspace before editing." {
		t.Fatalf("reasoning summary = %q", got.reasoningSummary)
	}
	view := got.View().Content
	reasoningIndex := strings.Index(view, "REASONING")
	inputIndex := strings.Index(view, "sk gode to work on this repo")
	if reasoningIndex < 0 || inputIndex < 0 || reasoningIndex > inputIndex {
		t.Fatalf("reasoning summary should render above composer:\n%s", view)
	}
	if !strings.Contains(view, "Checking workspace before editing.") {
		t.Fatalf("view missing reasoning summary:\n%s", view)
	}
}

func TestModelScrollState(t *testing.T) {
	model := New(nil)
	model.width = 40
	model.height = 10
	for i := 0; i < 10; i++ {
		model.addMessage("user", "", "message")
	}

	model.scrollBy(3)
	if model.scrollOffset != 3 || model.followTail {
		t.Fatalf("scrollOffset=%d followTail=%v", model.scrollOffset, model.followTail)
	}

	model.follow()
	if model.scrollOffset != 0 || !model.followTail {
		t.Fatalf("scrollOffset=%d followTail=%v", model.scrollOffset, model.followTail)
	}
}

func TestNewModelFocusesComposer(t *testing.T) {
	model := New(nil)
	if !model.input.Focused() {
		t.Fatal("composer should be focused")
	}
}

func TestComposerDoesNotPaintCursorLineBackground(t *testing.T) {
	model := New(nil)
	view := model.input.View()
	if strings.Contains(view, "\x1b[40m") || strings.Contains(view, "\x1b[48;5;0m") {
		t.Fatalf("composer view contains black background ANSI: %q", view)
	}
}

func TestCtrlPOpensSettingsDialog(t *testing.T) {
	model := New(nil)
	updated, _ := model.Update(keyCtrlP())
	got := updated.(Model)

	if !got.settings.open {
		t.Fatal("settings dialog should be open")
	}
	if got.input.Focused() {
		t.Fatal("composer should blur while settings dialog is open")
	}
	view := got.View().Content
	for _, want := range []string{"Settings", "Models", "Fast Mode", "Codex Sign In", "Config"} {
		if !strings.Contains(view, want) {
			t.Fatalf("view should render settings menu %q:\n%s", want, view)
		}
	}
}

func TestCtrlLTogglesErrorLogBelowComposer(t *testing.T) {
	model := New(nil)
	updated, _ := model.Update(runDoneMsg{Err: errors.New(`POST "https://chatgpt.com/backend-api/codex/responses": 400 Bad Request`)})
	got := updated.(Model)

	if len(got.errorLog) != 1 {
		t.Fatalf("error log length = %d", len(got.errorLog))
	}
	if got.showErrorLog {
		t.Fatal("error log should stay hidden until ctrl+l")
	}
	if got.status != "run failed - ctrl+l errors" {
		t.Fatalf("status = %q", got.status)
	}

	updated, _ = got.Update(keyCtrlL())
	got = updated.(Model)
	if !got.showErrorLog {
		t.Fatal("error log should be visible")
	}
	view := got.View().Content
	for _, want := range []string{"ERROR LOG", "400 Bad Request"} {
		if !strings.Contains(view, want) {
			t.Fatalf("view should render error log %q:\n%s", want, view)
		}
	}
}

func TestRunDoneDoesNotDuplicateRunFailedEventError(t *testing.T) {
	const message = `POST "https://chatgpt.com/backend-api/codex/responses": 400 Bad Request`
	model := New(nil)
	updated, _ := model.Update(eventMsg{Event: eventbus.Event{Kind: eventbus.KindRunFailed, Payload: map[string]any{"error": message}}})
	updated, _ = updated.Update(runDoneMsg{Err: errors.New(message)})
	got := updated.(Model)

	if len(got.messages) != 1 {
		t.Fatalf("messages length = %d, want 1: %#v", len(got.messages), got.messages)
	}
	if len(got.errorLog) != 1 {
		t.Fatalf("error log length = %d, want 1: %#v", len(got.errorLog), got.errorLog)
	}
}

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
	model.settings.menuIndex = 2

	updated, cmd := model.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	got := updated.(Model)
	if got.settings.open {
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

	if got.settings.screen != settingsScreenModels {
		t.Fatalf("settings screen = %v, want models", got.settings.screen)
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

	if got.settings.screen != settingsScreenModels {
		t.Fatalf("settings screen = %v, want models", got.settings.screen)
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
	model.settings.menuIndex = 1
	updated, _ := model.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	got := updated.(Model)

	if !got.settings.open {
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

	if got.settings.screen != settingsScreenReasoning {
		t.Fatalf("settings screen = %v, want reasoning", got.settings.screen)
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
	if got.settings.open {
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
	if got.settings.screen != settingsScreenReasoning {
		t.Fatalf("settings screen = %v, want reasoning", got.settings.screen)
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
	model.settings.openModels()
	_ = model.View()

	wantModel := godex.BuiltInModels(false)[1].ID
	click := clickZone(t, model, viewmodel.SettingsModelZoneID(wantModel))
	updated, _ := model.Update(click)
	got := updated.(Model)

	if got.settings.screen != settingsScreenReasoning {
		t.Fatalf("settings screen = %v, want reasoning", got.settings.screen)
	}
	_ = got.View()
	click = clickZone(t, got, viewmodel.SettingsReasoningZoneID(godex.ReasoningHigh))
	updated, _ = got.Update(click)
	got = updated.(Model)

	if got.settings.open {
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
	model.settings.models = []godex.ModelConfig{
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
	model.settings.openModels()

	updated, _ := model.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	got := updated.(Model)

	if got.settings.screen != settingsScreenReasoning {
		t.Fatalf("settings screen = %v, want reasoning", got.settings.screen)
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

	if !got.settings.open {
		t.Fatal("settings dialog should stay open")
	}
	if got.settings.screen != settingsScreenMenu {
		t.Fatalf("settings screen = %v, want menu", got.settings.screen)
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
	model.settings.openModels()
	model.settings.modelIndex = 1
	updated, _ := model.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	updated, _ = updated.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	got := updated.(Model)

	wantModel := godex.BuiltInModels(false)[1].ID
	if got.settings.open {
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

func keyCtrlL() tea.KeyPressMsg {
	return tea.KeyPressMsg{Code: 'l', Mod: tea.ModCtrl}
}
