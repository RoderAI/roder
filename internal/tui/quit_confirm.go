package tui

import (
	tea "charm.land/bubbletea/v2"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

func (m Model) updateQuitConfirm(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	switch msg.String() {
	case "enter":
		m.cancelEvents()
		return m, tea.Quit
	case "right", "esc", "escape", "ctrl+[":
		m.quitConfirmOpen = false
		m.status = "ready"
		return m, m.input.Focus()
	default:
		return m, nil
	}
}

func (m Model) quitConfirmViewModel() *viewmodel.ConfirmDialog {
	if !m.quitConfirmOpen {
		return nil
	}
	return &viewmodel.ConfirmDialog{
		Title:        "Quit gode?",
		Message:      "Are you sure you want to quit?",
		ConfirmLabel: "Enter quit",
		CancelLabel:  "No",
		Help:         "enter quit  right no  esc stay",
	}
}
