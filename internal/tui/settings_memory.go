package tui

import (
	"context"
	"fmt"

	tea "charm.land/bubbletea/v2"
	"github.com/pandelisz/gode/internal/godex/memory"
)

func (m *Model) refreshSettingsMemory() {
	m.settings.MemoryCount = 0
	if m.app == nil || m.app.Memory == nil {
		return
	}
	stats, err := m.app.Memory.Stats(context.Background())
	if err != nil {
		m.settings.Err = err.Error()
		return
	}
	m.settings.MemoryCount = stats.Count
	m.settings.Config.Memories = m.app.Config.Memories
	m.settings.Config.Memories.DatabasePath = stats.DatabasePath
	m.settings.Config.Memories.EmbeddingModel = stats.EmbeddingModel
	m.settings.Config.Memories.RecallLimit = stats.RecallLimit
}

func (m Model) toggleSelectedMemorySetting() (tea.Model, tea.Cmd) {
	switch m.settings.SelectedMemoryID() {
	case "enabled":
		return m, m.toggleMemoriesEnabled()
	default:
		return m, nil
	}
}

func (m *Model) toggleMemoriesEnabled() tea.Cmd {
	if m.running {
		m.settings.Err = "finish the current run before changing memories"
		return nil
	}
	next := !m.settings.Config.Memories.Enabled
	if m.app != nil {
		if err := m.app.SetMemoriesEnabled(next); err != nil {
			m.settings.Err = err.Error()
			return nil
		}
		if err := saveSettingsFromConfig(m.app.Config.DataDir, m.app.Config); err != nil {
			m.settings.Err = fmt.Sprintf("save settings: %v", err)
			return nil
		}
		m.settings.Config = m.app.Config
	} else {
		m.settings.Config.Memories.Enabled = next
	}
	m.refreshSettingsMemory()
	m.status = "memories " + onOff(next)
	return nil
}

func memorySettingsFromConfig(cfg memory.Config) memory.Settings {
	enabled := cfg.Enabled
	autoRecall := cfg.AutoRecall
	autoObserve := cfg.AutoObserve
	return memory.Settings{
		Enabled:        &enabled,
		AutoRecall:     &autoRecall,
		AutoObserve:    &autoObserve,
		EmbeddingModel: cfg.EmbeddingModel,
		RecallLimit:    cfg.RecallLimit,
		DatabasePath:   cfg.DatabasePath,
	}
}
