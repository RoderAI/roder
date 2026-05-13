package tui

import (
	"context"
	"strconv"
	"strings"

	tea "charm.land/bubbletea/v2"
	"github.com/google/uuid"
	"github.com/pandelisz/gode/internal/godex/agent"
	"github.com/pandelisz/gode/internal/godex/provider"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

type pendingPrompt struct {
	Display       string
	Prompt        string
	InputItems    []provider.Item
	ReplacePrompt bool
}

func (m *Model) preparePrompt(prompt string) (pendingPrompt, error) {
	pending := pendingPrompt{Display: strings.TrimSpace(prompt), Prompt: strings.TrimSpace(prompt)}
	if len(m.attachments) == 0 {
		return pending, nil
	}
	withAttachments, err := m.promptInputWithAttachments(prompt)
	if err != nil {
		return pendingPrompt{}, err
	}
	pending.Prompt = withAttachments.Prompt
	pending.InputItems = withAttachments.Items
	pending.ReplacePrompt = withAttachments.ReplacePrompt
	return pending, nil
}

func (m *Model) ensureSessionID() string {
	if strings.TrimSpace(m.currentSessionID) == "" {
		m.setCurrentSession(uuidString())
	}
	return m.currentSessionID
}

func uuidString() string {
	return uuid.NewString()
}

func (m *Model) submitPreparedPrompt(pending pendingPrompt) tea.Cmd {
	m.ensureSessionID()
	m.addMessage(viewmodel.RoleUser, "", pending.Display)
	m.reasoningSummary = ""
	m.attachments = nil
	m.input.Reset()
	m.running = true
	m.status = "waiting for model"
	return m.runPreparedPrompt(pending)
}

func (m *Model) steerPreparedPrompt(pending pendingPrompt) tea.Cmd {
	if pending.ReplacePrompt {
		return m.queuePreparedPrompt(pending)
	}
	m.ensureSessionID()
	m.addMessage(viewmodel.RoleUser, "steer", pending.Display)
	m.reasoningSummary = ""
	m.attachments = nil
	m.input.Reset()
	m.status = "steer queued for active run"
	return m.steerPrompt(pending.Prompt)
}

func (m *Model) queuePreparedPrompt(pending pendingPrompt) tea.Cmd {
	m.queuedPrompts = append(m.queuedPrompts, pending)
	m.attachments = nil
	m.input.Reset()
	m.status = queueStatus(len(m.queuedPrompts))
	return nil
}

func (m Model) queuedPromptDisplays() []string {
	out := make([]string, 0, len(m.queuedPrompts))
	for _, pending := range m.queuedPrompts {
		if display := strings.TrimSpace(pending.Display); display != "" {
			out = append(out, display)
		}
	}
	return out
}

func (m *Model) submitNextQueuedPrompt() tea.Cmd {
	if len(m.queuedPrompts) == 0 {
		return nil
	}
	next := m.queuedPrompts[0]
	copy(m.queuedPrompts, m.queuedPrompts[1:])
	m.queuedPrompts = m.queuedPrompts[:len(m.queuedPrompts)-1]
	return m.submitPreparedPrompt(next)
}

func queueStatus(count int) string {
	if count == 1 {
		return "queued 1 prompt"
	}
	return "queued " + strconv.Itoa(count) + " prompts"
}

func (m Model) steerPrompt(prompt string) tea.Cmd {
	return func() tea.Msg {
		if m.app == nil {
			return steerDoneMsg{Err: agent.ErrNoActiveRun}
		}
		runID, err := m.app.Steer(context.Background(), agent.SteerRequest{SessionID: m.currentSessionID, Prompt: prompt})
		return steerDoneMsg{RunID: runID, Err: err}
	}
}
