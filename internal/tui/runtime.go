package tui

import (
	"context"

	tea "charm.land/bubbletea/v2"
	"github.com/pandelisz/gode/internal/godex/agent"
	"github.com/pandelisz/gode/internal/godex/eventbus"
)

func (m Model) waitForEvent() tea.Cmd {
	if m.eventCh == nil {
		return nil
	}
	return func() tea.Msg {
		ev, ok := <-m.eventCh
		if !ok {
			return runDoneMsg{Err: eventbus.ErrClosed}
		}
		return eventMsg{Event: ev}
	}
}

func (m *Model) cancelEvents() {
	if m.eventCancel != nil {
		m.eventCancel()
		m.eventCancel = nil
	}
}

func (m Model) runPrompt(prompt string) tea.Cmd {
	return func() tea.Msg {
		var result agent.RunResult
		var err error
		if m.currentSessionID != "" {
			result, err = m.app.Run(context.Background(), agent.RunRequest{SessionID: m.currentSessionID, Prompt: prompt, Resume: true})
		} else {
			result, err = m.app.RunPrompt(context.Background(), prompt)
		}
		return runDoneMsg{Result: result, Err: err}
	}
}
