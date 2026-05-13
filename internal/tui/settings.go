package tui

import (
	"context"
	"fmt"

	tea "charm.land/bubbletea/v2"
	"github.com/pandelisz/gode/internal/godex"
	"github.com/pandelisz/gode/internal/godex/codexauth"
	"github.com/pandelisz/gode/internal/tui/dialogs"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

func (m *Model) openSettings() {
	cfg := godex.DefaultConfig()
	if m.app != nil {
		cfg = m.app.Config
	}
	m.settings = dialogs.NewSettings(cfg)
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
		}
		return m, nil
	case dialogs.ScreenModels:
		return m.selectSettingsModel()
	case dialogs.ScreenReasoning:
		return m, m.saveSelectedModelReasoning()
	case dialogs.ScreenConfig:
		m.settings.OpenMenu()
		return m, nil
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
	if reasoning == "" {
		m.settings.Err = "reasoning is required"
		return m, nil
	}
	if m.app != nil {
		if err := m.app.SetModelReasoning(selected.ID, reasoning); err != nil {
			m.settings.Err = err.Error()
			return m, nil
		}
		if err := godex.SaveSettings(m.app.Config.DataDir, settingsFromConfig(m.app.Config)); err != nil {
			m.settings.Err = fmt.Sprintf("save settings: %v", err)
			return m, nil
		}
		m.settings.Config = m.app.Config
	} else {
		m.settings.Config.Model = selected.ID
		m.settings.Config.Provider = selected.Provider
		m.settings.Config.Reasoning = reasoning
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
		if err := godex.SaveSettings(m.app.Config.DataDir, settingsFromConfig(m.app.Config)); err != nil {
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
		if err := godex.SaveSettings(m.app.Config.DataDir, settingsFromConfig(m.app.Config)); err != nil {
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
