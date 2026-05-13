package tui

import (
	"context"
	"fmt"

	tea "charm.land/bubbletea/v2"
	"github.com/pandelisz/gode/internal/godex"
	"github.com/pandelisz/gode/internal/godex/codexauth"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

type settingsScreen int

const (
	settingsScreenMenu settingsScreen = iota
	settingsScreenModels
	settingsScreenReasoning
	settingsScreenConfig
)

type settingsDialog struct {
	open           bool
	screen         settingsScreen
	cfg            godex.Config
	models         []godex.ModelConfig
	menuIndex      int
	modelIndex     int
	reasoningIndex int
	err            string
}

func newSettingsDialog(cfg godex.Config) settingsDialog {
	settings := settingsDialog{
		open:   true,
		screen: settingsScreenMenu,
		cfg:    cfg,
		models: godex.BuiltInModels(false),
	}
	settings.selectCurrentModel()
	return settings
}

func (m *Model) openSettings() {
	cfg := godex.DefaultConfig()
	if m.app != nil {
		cfg = m.app.Config
	}
	m.settings = newSettingsDialog(cfg)
	m.input.Blur()
	m.status = "settings"
}

func (m *Model) resizeSettings() {}

func (m *Model) closeSettings(status string) tea.Cmd {
	m.settings = settingsDialog{}
	m.status = status
	return m.input.Focus()
}

func (m Model) updateSettings(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	switch msg.String() {
	case "ctrl+c":
		return m, tea.Quit
	case "ctrl+p":
		return m, m.closeSettings("ready")
	case "esc":
		if m.settings.screen == settingsScreenMenu {
			return m, m.closeSettings("ready")
		}
		if m.settings.screen == settingsScreenReasoning {
			m.settings.backToModels()
			return m, nil
		}
		m.settings.openMenu()
		return m, nil
	case "left", "backspace":
		if m.settings.screen == settingsScreenReasoning {
			m.settings.backToModels()
		} else if m.settings.screen != settingsScreenMenu {
			m.settings.openMenu()
		}
		return m, nil
	case "right":
		return m.activateSettingsSelection()
	case "enter":
		return m.activateSettingsSelection()
	case "down", "j":
		m.settings.move(1)
		return m, nil
	case "up", "k":
		m.settings.move(-1)
		return m, nil
	}
	return m, nil
}

func (m Model) updateSettingsMouse(msg tea.MouseClickMsg) (tea.Model, tea.Cmd) {
	switch m.settings.screen {
	case settingsScreenMenu:
		for i, item := range m.settings.menuItems() {
			z := m.zones.Get(viewmodel.SettingsMenuItemZoneID(item.ID))
			if z != nil && z.InBounds(msg) {
				m.settings.menuIndex = i
				return m.activateSettingsSelection()
			}
		}
	case settingsScreenModels:
		for i, model := range m.settings.models {
			z := m.zones.Get(viewmodel.SettingsModelZoneID(model.ID))
			if z != nil && z.InBounds(msg) {
				m.settings.modelIndex = i
				m.settings.openReasoning()
				return m, nil
			}
		}
	case settingsScreenReasoning:
		for i, option := range m.settings.reasoningOptions() {
			z := m.zones.Get(viewmodel.SettingsReasoningZoneID(option.Effort))
			if z != nil && z.InBounds(msg) {
				m.settings.reasoningIndex = i
				return m, m.saveSelectedModelReasoning()
			}
		}
	}
	return m, nil
}

func (m Model) activateSettingsSelection() (tea.Model, tea.Cmd) {
	switch m.settings.screen {
	case settingsScreenMenu:
		switch m.settings.selectedMenuID() {
		case "models":
			m.settings.openModels()
		case "fast-mode":
			return m, m.toggleFastMode()
		case "config":
			m.settings.openConfig()
		case "codex-auth":
			return m, m.startCodexSignIn()
		}
		return m, nil
	case settingsScreenModels:
		m.settings.openReasoning()
		return m, nil
	case settingsScreenReasoning:
		return m, m.saveSelectedModelReasoning()
	case settingsScreenConfig:
		m.settings.openMenu()
		return m, nil
	default:
		return m, nil
	}
}

func (m *Model) startCodexSignIn() tea.Cmd {
	cfg := m.settings.cfg
	if cfg.DataDir == "" {
		cfg = godex.DefaultConfig()
	}
	m.settings = settingsDialog{}
	m.status = "opening browser for codex sign-in"
	_ = m.input.Focus()
	login := m.codexLogin
	if login == nil {
		login = codexauth.LoginBrowser
	}
	return func() tea.Msg {
		tokens, _, err := login(context.Background(), cfg.DataDir)
		return codexAuthDoneMsg{AccountID: tokens.AccountID, Err: err}
	}
}

func (m *Model) saveSelectedModelReasoning() tea.Cmd {
	if len(m.settings.models) == 0 {
		m.settings.err = "no models available"
		return nil
	}
	if m.settings.modelIndex < 0 || m.settings.modelIndex >= len(m.settings.models) {
		m.settings.modelIndex = 0
	}
	selected := m.settings.models[m.settings.modelIndex]
	if selected.ID == "" {
		m.settings.err = "model is required"
		return nil
	}
	if m.running {
		m.settings.err = "finish the current run before changing models"
		return nil
	}
	reasoning := m.settings.selectedReasoningEffort()
	if reasoning == "" {
		m.settings.err = "reasoning is required"
		return nil
	}
	if m.app != nil {
		if err := m.app.SetModelReasoning(selected.ID, reasoning); err != nil {
			m.settings.err = err.Error()
			return nil
		}
		if err := godex.SaveSettings(m.app.Config.DataDir, settingsFromConfig(m.app.Config)); err != nil {
			m.settings.err = fmt.Sprintf("save settings: %v", err)
			return nil
		}
		m.settings.cfg = m.app.Config
	}
	return m.closeSettings("default model saved")
}

func (m *Model) toggleFastMode() tea.Cmd {
	if m.running {
		m.settings.err = "finish the current run before changing fast mode"
		return nil
	}
	next := !m.settings.cfg.FastMode
	if m.app != nil {
		if err := m.app.SetFastMode(next); err != nil {
			m.settings.err = err.Error()
			return nil
		}
		if err := godex.SaveSettings(m.app.Config.DataDir, settingsFromConfig(m.app.Config)); err != nil {
			m.settings.err = fmt.Sprintf("save settings: %v", err)
			return nil
		}
		m.settings.cfg = m.app.Config
	} else {
		m.settings.cfg.FastMode = next
	}
	m.status = "fast mode " + onOff(next)
	return nil
}

func (s *settingsDialog) openMenu() {
	s.screen = settingsScreenMenu
	s.err = ""
}

func (s *settingsDialog) openModels() {
	s.screen = settingsScreenModels
	s.err = ""
	s.selectCurrentModel()
}

func (s *settingsDialog) backToModels() {
	s.screen = settingsScreenModels
	s.err = ""
}

func (s *settingsDialog) openReasoning() {
	s.screen = settingsScreenReasoning
	s.err = ""
	s.selectCurrentReasoning()
}

func (s *settingsDialog) openConfig() {
	s.screen = settingsScreenConfig
	s.err = ""
}

func (s *settingsDialog) move(delta int) {
	count := s.selectionCount()
	if count == 0 {
		return
	}
	switch s.screen {
	case settingsScreenMenu:
		s.menuIndex = wrapIndex(s.menuIndex+delta, count)
	case settingsScreenModels:
		s.modelIndex = wrapIndex(s.modelIndex+delta, count)
	case settingsScreenReasoning:
		s.reasoningIndex = wrapIndex(s.reasoningIndex+delta, count)
	}
}

func (s settingsDialog) selectionCount() int {
	switch s.screen {
	case settingsScreenMenu:
		return len(s.menuItems())
	case settingsScreenModels:
		return len(s.models)
	case settingsScreenReasoning:
		return len(s.reasoningOptions())
	default:
		return 0
	}
}

func (s settingsDialog) selectedMenuID() string {
	items := s.menuItems()
	if len(items) == 0 {
		return ""
	}
	index := clamp(s.menuIndex, 0, len(items)-1)
	return items[index].ID
}

func (s *settingsDialog) selectCurrentModel() {
	s.modelIndex = 0
	for i, model := range s.models {
		if model.ID == s.cfg.Model {
			s.modelIndex = i
			return
		}
	}
}

func (s *settingsDialog) selectCurrentReasoning() {
	s.reasoningIndex = 0
	options := s.reasoningOptions()
	selectedModel := s.selectedModel()
	preferred := s.cfg.Reasoning
	if preferred == "" || !selectedModel.SupportsReasoning(preferred) {
		preferred = selectedModel.DefaultReasoning
	}
	for i, option := range options {
		if option.Effort == preferred {
			s.reasoningIndex = i
			return
		}
	}
}

func (s settingsDialog) selectedModel() godex.ModelConfig {
	if len(s.models) == 0 {
		return godex.DefaultModelConfig()
	}
	index := clamp(s.modelIndex, 0, len(s.models)-1)
	return s.models[index]
}

func (s settingsDialog) reasoningOptions() []godex.ReasoningOption {
	model := s.selectedModel()
	if len(model.SupportedReasoning) == 0 {
		return []godex.ReasoningOption{{Effort: model.DefaultReasoning}}
	}
	return model.SupportedReasoning
}

func (s settingsDialog) selectedReasoningEffort() string {
	options := s.reasoningOptions()
	if len(options) == 0 {
		return ""
	}
	index := clamp(s.reasoningIndex, 0, len(options)-1)
	return options[index].Effort
}

func (s settingsDialog) menuItems() []viewmodel.SettingsMenuItem {
	codexStatus := "signed out"
	if (codexauth.Store{DataDir: s.cfg.DataDir}).SignedIn() {
		codexStatus = "signed in"
	}
	return []viewmodel.SettingsMenuItem{
		{
			ID:          "models",
			Label:       "Models",
			Description: "Choose the default model for new gode sessions.",
			Value:       godex.DisplayModelLabel(s.cfg) + " / " + s.cfg.Reasoning,
		},
		{
			ID:          "fast-mode",
			Label:       "Fast Mode",
			Description: "Use OpenAI priority processing for model requests.",
			Value:       onOff(s.cfg.FastMode),
		},
		{
			ID:          "codex-auth",
			Label:       "Codex Sign In",
			Description: "Connect ChatGPT Codex so GPT models use Codex auth.",
			Value:       codexStatus,
		},
		{
			ID:          "config",
			Label:       "Config",
			Description: "Review provider, reasoning, workspace, and data paths.",
			Value:       godex.DisplayProvider(s.cfg),
		},
	}
}

func (s settingsDialog) viewModel() *viewmodel.SettingsDialog {
	if !s.open {
		return nil
	}

	vm := &viewmodel.SettingsDialog{
		Title:      s.title(),
		Screen:     s.screenName(),
		MenuItems:  s.viewMenuItems(),
		Models:     s.viewModels(),
		Reasoning:  s.viewReasoning(),
		ConfigRows: s.configRows(),
		Selected:   s.selectedIndex(),
		Error:      s.err,
	}
	return vm
}

func (s settingsDialog) title() string {
	switch s.screen {
	case settingsScreenModels:
		return "Models"
	case settingsScreenReasoning:
		return "Reasoning"
	case settingsScreenConfig:
		return "Config"
	default:
		return "Settings"
	}
}

func (s settingsDialog) screenName() string {
	switch s.screen {
	case settingsScreenModels:
		return viewmodel.SettingsScreenModels
	case settingsScreenReasoning:
		return viewmodel.SettingsScreenReasoning
	case settingsScreenConfig:
		return viewmodel.SettingsScreenConfig
	default:
		return viewmodel.SettingsScreenMenu
	}
}

func (s settingsDialog) selectedIndex() int {
	switch s.screen {
	case settingsScreenModels:
		return s.modelIndex
	case settingsScreenReasoning:
		return s.reasoningIndex
	default:
		return s.menuIndex
	}
}

func (s settingsDialog) viewMenuItems() []viewmodel.SettingsMenuItem {
	items := s.menuItems()
	for i := range items {
		items[i].Selected = i == s.menuIndex
	}
	return items
}

func (s settingsDialog) viewModels() []viewmodel.SettingsModelItem {
	items := make([]viewmodel.SettingsModelItem, 0, len(s.models))
	for i, model := range s.models {
		items = append(items, viewmodel.SettingsModelItem{
			ID:               model.ID,
			DisplayName:      model.DisplayName,
			Description:      model.Description,
			Provider:         model.Provider,
			DefaultReasoning: model.DefaultReasoning,
			Current:          model.ID == s.cfg.Model,
			Selected:         i == s.modelIndex,
		})
	}
	return items
}

func (s settingsDialog) viewReasoning() []viewmodel.SettingsReasoningItem {
	options := s.reasoningOptions()
	items := make([]viewmodel.SettingsReasoningItem, 0, len(options))
	selectedModel := s.selectedModel()
	current := ""
	if s.cfg.Model == selectedModel.ID {
		current = s.cfg.Reasoning
	}
	for i, option := range options {
		items = append(items, viewmodel.SettingsReasoningItem{
			Effort:      option.Effort,
			Label:       reasoningLabel(option.Effort),
			Description: option.Description,
			Current:     option.Effort == current,
			Selected:    i == s.reasoningIndex,
		})
	}
	return items
}

func (s settingsDialog) configRows() []viewmodel.SettingsConfigRow {
	return []viewmodel.SettingsConfigRow{
		{Label: "Model", Value: s.cfg.Model},
		{Label: "Provider", Value: godex.DisplayProvider(s.cfg)},
		{Label: "Reasoning", Value: s.cfg.Reasoning},
		{Label: "Fast mode", Value: onOff(s.cfg.FastMode)},
		{Label: "Workspace", Value: s.cfg.Workspace},
		{Label: "Data dir", Value: s.cfg.DataDir},
	}
}

func settingsFromConfig(cfg godex.Config) godex.Settings {
	return godex.Settings{
		DefaultModel:     cfg.Model,
		DefaultReasoning: cfg.Reasoning,
		FastMode:         cfg.FastMode,
	}
}

func onOff(enabled bool) string {
	if enabled {
		return "on"
	}
	return "off"
}

func reasoningLabel(effort string) string {
	switch effort {
	case godex.ReasoningNone:
		return "None"
	case godex.ReasoningMinimal:
		return "Minimal"
	case godex.ReasoningLow:
		return "Low"
	case godex.ReasoningMedium:
		return "Medium"
	case godex.ReasoningHigh:
		return "High"
	case godex.ReasoningXHigh:
		return "XHigh"
	default:
		return effort
	}
}

func wrapIndex(index int, count int) int {
	if count <= 0 {
		return 0
	}
	index %= count
	if index < 0 {
		index += count
	}
	return index
}
