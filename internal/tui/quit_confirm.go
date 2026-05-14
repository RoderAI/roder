package tui

import (
	"context"

	tea "charm.land/bubbletea/v2"
	"github.com/pandelisz/gode/internal/godex/eventbus"
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

func (m Model) updateStopConfirm(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	switch msg.String() {
	case "enter":
		if m.runCancel != nil {
			m.runCancel()
		}
		if m.app != nil && m.app.Bus != nil {
			m.app.Bus.Publish(context.Background(), eventbus.Event{
				Kind:      eventbus.KindRunCancelRequested,
				Source:    eventbus.SourceTUI,
				SessionID: m.currentSessionID,
				RunID:     m.currentRunID,
			})
		}
		m.stopConfirmOpen = false
		m.status = "stopping run"
		return m, m.input.Focus()
	case "right", "esc", "escape", "ctrl+[":
		m.stopConfirmOpen = false
		m.status = "running"
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

func (m Model) stopConfirmViewModel() *viewmodel.ConfirmDialog {
	if !m.stopConfirmOpen {
		return nil
	}
	return &viewmodel.ConfirmDialog{
		Title:        "Stop current turn?",
		Message:      "Are you sure you want to stop the active run?",
		ConfirmLabel: "Enter stop",
		CancelLabel:  "No",
		Help:         "enter stop  right no  esc stay",
	}
}
