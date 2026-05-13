package tui

import (
	"context"
	"fmt"

	tea "charm.land/bubbletea/v2"
	"github.com/pandelisz/gode/internal/godex"
	"github.com/pandelisz/gode/internal/godex/codexauth"
	"github.com/pandelisz/gode/internal/tui/dialogs"
	tuiskills "github.com/pandelisz/gode/internal/tui/skills"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

func (m *Model) openSettings() {
	cfg := godex.DefaultConfig()
	if m.app != nil {
		cfg = m.app.Config
	}
	m.settings = dialogs.NewSettings(cfg)
	m.refreshSettingsSkills()
	m.input.Blur()
	m.status = "settings"
}

func (m *Model) resizeSettings() {}

func (m *Model) closeSettings(status string) tea.Cmd {
	m.settings = dialogs.Settings{}
	m.status = status
	return m.input.Focus()
}

func (m Model) updateSettings(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	switch msg.String() {
	case "ctrl+c":
		return m, tea.Quit
	case "ctrl+p":
		return m, m.closeSettings("ready")
	case "esc", "escape", "ctrl+[":
		if m.settings.Screen == dialogs.ScreenMenu {
			return m, m.closeSettings("ready")
		}
		if m.settings.Screen == dialogs.ScreenReasoning {
			m.settings.BackToModels()
			return m, nil
		}
		m.settings.OpenMenu()
		return m, nil
	case "left", "backspace":
		if m.settings.Screen == dialogs.ScreenReasoning {
			m.settings.BackToModels()
		} else if m.settings.Screen != dialogs.ScreenMenu {
			m.settings.OpenMenu()
		}
		return m, nil
	case "right", "enter":
		return m.activateSettingsSelection()
	case " ", "space":
		if m.settings.Screen == dialogs.ScreenSkills {
			return m.toggleSelectedSkill()
		}
		return m, nil
	case "r":
		if m.settings.Screen == dialogs.ScreenSkills {
			m.settings.OpenSkillRecommendations()
			return m, nil
		}
		return m, nil
	case "a":
		if m.settings.Screen == dialogs.ScreenSkillRecommendations {
			return m, m.installMissingRecommendedSkills()
		}
		return m, nil
	case "down", "j":
		m.settings.Move(1)
		return m, nil
	case "up", "k":
		m.settings.Move(-1)
		return m, nil
	}
	return m, nil
}

func (m Model) updateSettingsMouse(msg tea.MouseClickMsg) (tea.Model, tea.Cmd) {
	switch m.settings.Screen {
	case dialogs.ScreenMenu:
		for i, item := range m.settings.MenuItems() {
			z := m.zones.Get(viewmodel.SettingsMenuItemZoneID(item.ID))
			if z != nil && z.InBounds(msg) {
				m.settings.MenuIndex = i
				return m.activateSettingsSelection()
			}
		}
	case dialogs.ScreenModels:
		for i, model := range m.settings.Models {
			z := m.zones.Get(viewmodel.SettingsModelZoneID(model.ID))
			if z != nil && z.InBounds(msg) {
				m.settings.ModelIndex = i
				return m.selectSettingsModel()
			}
		}
	case dialogs.ScreenReasoning:
		for i, option := range m.settings.ReasoningOptions() {
			z := m.zones.Get(viewmodel.SettingsReasoningZoneID(option.Effort))
			if z != nil && z.InBounds(msg) {
				m.settings.ReasoningIndex = i
				return m, m.saveSelectedModelReasoning()
			}
		}
	case dialogs.ScreenSkills:
		for i, item := range m.settings.Skills {
			z := m.zones.Get(viewmodel.SettingsSkillZoneID(item.Name))
			if z != nil && z.InBounds(msg) {
				m.settings.SkillIndex = i
				return m.toggleSelectedSkill()
			}
		}
	case dialogs.ScreenSkillRecommendations:
		for i, item := range m.settings.RecommendedSkills {
			z := m.zones.Get(viewmodel.SettingsRecommendedSkillZoneID(item.Name))
			if z != nil && z.InBounds(msg) {
				m.settings.RecommendedIndex = i
				return m, nil
			}
		}
	}
	return m, nil
}

func (m Model) activateSettingsSelection() (tea.Model, tea.Cmd) {
	switch m.settings.Screen {
	case dialogs.ScreenMenu:
		switch m.settings.SelectedMenuID() {
		case "models":
			m.settings.OpenModels()
		case "fast-mode":
			return m, m.toggleFastMode()
		case "config":
			m.settings.OpenConfig()
		case "codex-auth":
			return m, m.startCodexSignIn()
		case "skills":
			m.settings.OpenSkills()
		}
		return m, nil
	case dialogs.ScreenModels:
		return m.selectSettingsModel()
	case dialogs.ScreenReasoning:
		return m, m.saveSelectedModelReasoning()
	case dialogs.ScreenConfig:
		m.settings.OpenMenu()
		return m, nil
	case dialogs.ScreenSkills:
		return m.toggleSelectedSkill()
	case dialogs.ScreenSkillRecommendations:
		return m, m.installMissingRecommendedSkills()
	default:
		return m, nil
	}
}

func (m Model) selectSettingsModel() (tea.Model, tea.Cmd) {
	selected := m.settings.SelectedModel()
	if selected.ID == "" {
		m.settings.Err = "model is required"
		return m, nil
	}
	if m.running {
		m.settings.Err = "finish the current run before changing models"
		return m, nil
	}
	reasoning := m.settings.PreferredReasoning(selected)
	hasReasoning := len(selected.SupportedReasoning) > 0
	if hasReasoning && reasoning == "" {
		m.settings.Err = "reasoning is required"
		return m, nil
	}
	if m.app != nil {
		if err := m.app.SetModelReasoning(selected.ID, reasoning); err != nil {
			m.settings.Err = err.Error()
			return m, nil
		}
		if err := saveSettingsFromConfig(m.app.Config.DataDir, m.app.Config); err != nil {
			m.settings.Err = fmt.Sprintf("save settings: %v", err)
			return m, nil
		}
		m.settings.Config = m.app.Config
	} else {
		m.settings.Config.Model = selected.ID
		m.settings.Config.Provider = selected.Provider
		m.settings.Config.Reasoning = reasoning
	}
	if !hasReasoning {
		return m, m.closeSettings("default model saved")
	}
	m.settings.OpenReasoning()
	m.status = "default model selected"
	return m, nil
}

func (m *Model) startCodexSignIn() tea.Cmd {
	cfg := m.settings.Config
	if cfg.DataDir == "" {
		cfg = godex.DefaultConfig()
	}
	m.settings = dialogs.Settings{}
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
	if len(m.settings.Models) == 0 {
		m.settings.Err = "no models available"
		return nil
	}
	if m.settings.ModelIndex < 0 || m.settings.ModelIndex >= len(m.settings.Models) {
		m.settings.ModelIndex = 0
	}
	selected := m.settings.Models[m.settings.ModelIndex]
	if selected.ID == "" {
		m.settings.Err = "model is required"
		return nil
	}
	if m.running {
		m.settings.Err = "finish the current run before changing models"
		return nil
	}
	if len(selected.SupportedReasoning) == 0 {
		return m.closeSettings("default model saved")
	}
	reasoning := m.settings.SelectedReasoningEffort()
	if reasoning == "" {
		m.settings.Err = "reasoning is required"
		return nil
	}
	if m.app != nil {
		if err := m.app.SetModelReasoning(selected.ID, reasoning); err != nil {
			m.settings.Err = err.Error()
			return nil
		}
		if err := saveSettingsFromConfig(m.app.Config.DataDir, m.app.Config); err != nil {
			m.settings.Err = fmt.Sprintf("save settings: %v", err)
			return nil
		}
		m.settings.Config = m.app.Config
	}
	return m.closeSettings("default model saved")
}

func (m *Model) toggleFastMode() tea.Cmd {
	if m.running {
		m.settings.Err = "finish the current run before changing fast mode"
		return nil
	}
	next := !m.settings.Config.FastMode
	if m.app != nil {
		if err := m.app.SetFastMode(next); err != nil {
			m.settings.Err = err.Error()
			return nil
		}
		if err := saveSettingsFromConfig(m.app.Config.DataDir, m.app.Config); err != nil {
			m.settings.Err = fmt.Sprintf("save settings: %v", err)
			return nil
		}
		m.settings.Config = m.app.Config
	} else {
		m.settings.Config.FastMode = next
	}
	m.status = "fast mode " + onOff(next)
	return nil
}

func settingsFromConfig(cfg godex.Config) godex.Settings {
	return godex.Settings{
		DefaultModel:          cfg.Model,
		DefaultReasoning:      cfg.Reasoning,
		FastMode:              cfg.FastMode,
		DisableAutoCompaction: cfg.DisableAutoCompaction,
		AutoCompactTokenLimit: cfg.AutoCompactTokenLimit,
	}
}

func saveSettingsFromConfig(dataDir string, cfg godex.Config) error {
	settings, err := godex.LoadSettings(dataDir)
	if err != nil {
		return err
	}
	settings.DefaultModel = cfg.Model
	settings.DefaultReasoning = cfg.Reasoning
	settings.FastMode = cfg.FastMode
	settings.DisableAutoCompaction = cfg.DisableAutoCompaction
	settings.AutoCompactTokenLimit = cfg.AutoCompactTokenLimit
	return godex.SaveSettings(dataDir, settings)
}

func (m *Model) refreshSettingsSkills() {
	if m.app == nil || m.app.SkillManager == nil {
		m.settings.Skills = nil
		m.settings.RecommendedSkills = nil
		m.settings.SkillEnabledCount = 0
		m.settings.SkillInstalledCount = 0
		return
	}
	ctx := context.Background()
	installed, err := m.app.SkillManager.List(ctx)
	if err != nil {
		m.settings.Err = err.Error()
		return
	}
	recommended, err := m.app.SkillManager.Recommended(ctx)
	if err != nil {
		m.settings.Err = err.Error()
		return
	}
	state := tuiskills.ViewState(installed, recommended)
	m.settings.Skills = state.Installed
	m.settings.RecommendedSkills = state.Recommended
	m.settings.SkillInstalledCount = state.InstalledN
	m.settings.SkillEnabledCount = state.EnabledN
}

func (m Model) toggleSelectedSkill() (tea.Model, tea.Cmd) {
	item, ok := tuiskills.SelectedSkill(m.settings.Skills, m.settings.SkillIndex)
	if !ok || item.Name == "" {
		m.settings.Err = "no skill selected"
		return m, nil
	}
	if m.app == nil || m.app.SkillManager == nil {
		m.settings.Err = "skills manager unavailable"
		return m, nil
	}
	next := !item.Enabled
	if err := m.app.SkillManager.SetEnabled(context.Background(), item.Name, next); err != nil {
		m.settings.Err = err.Error()
		return m, nil
	}
	m.refreshSettingsSkills()
	m.status = fmt.Sprintf("skill %s %s", item.Name, onOff(next))
	return m, nil
}

func (m *Model) installMissingRecommendedSkills() tea.Cmd {
	if m.app == nil || m.app.SkillManager == nil {
		m.settings.Err = "skills manager unavailable"
		return nil
	}
	names := tuiskills.MissingRecommendedNames(m.settings.RecommendedSkills)
	if len(names) == 0 {
		m.status = "recommended skills already installed"
		return nil
	}
	manager := m.app.SkillManager
	m.status = fmt.Sprintf("installing %d recommended skills", len(names))
	return func() tea.Msg {
		results, err := manager.InstallRecommended(context.Background(), names)
		if err != nil {
			return skillsInstallDoneMsg{Err: err}
		}
		return skillsInstallDoneMsg{Installed: len(results)}
	}
}

func onOff(enabled bool) string {
	if enabled {
		return "on"
	}
	return "off"
}
