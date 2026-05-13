package tui

import (
	"context"
	"strings"

	tea "charm.land/bubbletea/v2"
	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

func (m *Model) handleCompactInput(prompt string) (bool, tea.Cmd) {
	if strings.TrimSpace(prompt) != "/compact" {
		return false, nil
	}
	m.input.Reset()
	if m.app == nil {
		m.addMessage(viewmodel.RoleError, "compact", "compact requires an app")
		m.status = "compact failed - ctrl+l errors"
		return true, nil
	}
	if m.running {
		m.addMessage(viewmodel.RoleError, "compact", "wait for the active run to finish before compacting")
		m.status = "compact failed - ctrl+l errors"
		return true, nil
	}
	sessionID := strings.TrimSpace(m.currentSessionID)
	if sessionID == "" {
		m.addMessage(viewmodel.RoleError, "compact", "compact requires an active session")
		m.status = "compact failed - ctrl+l errors"
		return true, nil
	}
	m.status = "compacting context"
	return true, m.compactSession(sessionID)
}

func (m Model) compactSession(sessionID string) tea.Cmd {
	return func() tea.Msg {
		result, err := m.app.CompactSession(context.Background(), sessionID)
		if err != nil {
			return eventMsg{Event: eventbus.Event{
				Kind:      eventbus.KindContextCompactionFailed,
				SessionID: sessionID,
				Payload:   map[string]any{"error": err.Error()},
			}}
		}
		return eventMsg{Event: eventbus.Event{
			Kind:      eventbus.KindContextCompactionCompleted,
			SessionID: result.SessionID,
			RunID:     result.RunID,
			Payload: map[string]any{
				"response_id":  result.ResponseID,
				"output_items": result.OutputItems,
			},
		}}
	}
}
