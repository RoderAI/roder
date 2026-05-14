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

func (m *Model) runPrompt(prompt string) tea.Cmd {
	runCtx, cancel := context.WithCancel(context.Background())
	m.runCancel = cancel
	if m.currentRunID == "" {
		m.currentRunID = uuidString()
	}
	return m.runPreparedPrompt(runCtx, pendingPrompt{Prompt: prompt, Display: prompt})
}

func (m Model) runPreparedPrompt(ctx context.Context, pending pendingPrompt) tea.Cmd {
	return func() tea.Msg {
		var result agent.RunResult
		var err error
		req := agent.RunRequest{
			SessionID:     m.currentSessionID,
			RunID:         m.currentRunID,
			Prompt:        pending.Prompt,
			Resume:        true,
			InputItems:    pending.InputItems,
			ReplacePrompt: pending.ReplacePrompt,
		}
		if m.currentSessionID != "" {
			result, err = m.app.Run(ctx, req)
		} else {
			result, err = m.app.RunPrompt(ctx, pending.Prompt)
		}
		return runDoneMsg{Result: result, Err: err}
	}
}
