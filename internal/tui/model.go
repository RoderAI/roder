package tui

import (
	"context"
	"fmt"
	"strings"

	"charm.land/bubbles/v2/textarea"
	tea "charm.land/bubbletea/v2"
	"github.com/pandelisz/gode/internal/godex"
	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/tui/components"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

type eventMsg struct {
	Event eventbus.Event
}

type runDoneMsg struct {
	Err error
}

type Model struct {
	app    *godex.App
	input  textarea.Model
	lines  []string
	width  int
	height int
}

func New(app *godex.App) Model {
	input := textarea.New()
	input.Placeholder = "Ask gode to work on this repo"
	input.Prompt = "> "
	input.ShowLineNumbers = false
	input.DynamicHeight = true
	input.MinHeight = 1
	input.MaxHeight = 6
	input.SetWidth(80)
	input.Focus()
	return Model{app: app, input: input}
}

func (m Model) Init() tea.Cmd {
	return m.waitForEvent()
}

func (m Model) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.WindowSizeMsg:
		m.width = msg.Width
		m.height = msg.Height
		m.input.SetWidth(max(20, msg.Width-4))
		return m, nil
	case tea.KeyPressMsg:
		switch msg.String() {
		case "ctrl+c", "esc":
			return m, tea.Quit
		case "enter":
			prompt := strings.TrimSpace(m.input.Value())
			if prompt == "" {
				return m, nil
			}
			m.lines = append(m.lines, "user: "+prompt)
			m.input.Reset()
			return m, tea.Batch(m.runPrompt(prompt), m.waitForEvent())
		}
	case eventMsg:
		m.appendEvent(msg.Event)
		return m, m.waitForEvent()
	case runDoneMsg:
		if msg.Err != nil {
			m.lines = append(m.lines, "error: "+msg.Err.Error())
		}
		return m, nil
	}

	var cmd tea.Cmd
	m.input, cmd = m.input.Update(msg)
	return m, cmd
}

func (m Model) View() tea.View {
	vm := viewmodel.Model{
		Width:       m.width,
		Height:      m.height,
		Lines:       m.lines,
		Input:       m.input.View(),
		InputHeight: m.input.Height(),
	}
	if m.app != nil {
		vm.Provider = m.app.Config.Provider
		vm.Model = m.app.Config.Model
	}
	return tea.NewView(components.Render(vm))
}

func (m Model) waitForEvent() tea.Cmd {
	if m.app == nil || m.app.Bus == nil {
		return nil
	}
	return func() tea.Msg {
		ev, err := m.app.Bus.Await(context.Background(), eventbus.Filter{})
		if err != nil {
			return runDoneMsg{Err: err}
		}
		return eventMsg{Event: ev}
	}
}

func (m Model) runPrompt(prompt string) tea.Cmd {
	return func() tea.Msg {
		_, err := m.app.RunPrompt(context.Background(), prompt)
		return runDoneMsg{Err: err}
	}
}

func (m *Model) appendEvent(ev eventbus.Event) {
	switch ev.Kind {
	case eventbus.KindAssistantDelta:
		var payload struct {
			Text string `json:"text"`
		}
		_ = ev.DecodePayload(&payload)
		if payload.Text != "" {
			m.lines = append(m.lines, "assistant: "+payload.Text)
		}
	case eventbus.KindToolRequested:
		var payload struct {
			Tool string `json:"tool"`
		}
		_ = ev.DecodePayload(&payload)
		m.lines = append(m.lines, "tool: "+payload.Tool)
	case eventbus.KindToolCompleted:
		var payload struct {
			Tool string `json:"tool"`
			Text string `json:"text"`
		}
		_ = ev.DecodePayload(&payload)
		m.lines = append(m.lines, fmt.Sprintf("tool result: %s %s", payload.Tool, payload.Text))
	case eventbus.KindPermissionRequested:
		var payload struct {
			Tool string `json:"tool"`
		}
		_ = ev.DecodePayload(&payload)
		m.lines = append(m.lines, "permission requested: "+payload.Tool)
	case eventbus.KindRunCompleted:
		m.lines = append(m.lines, "run completed")
	case eventbus.KindRunFailed:
		var payload struct {
			Error string `json:"error"`
		}
		_ = ev.DecodePayload(&payload)
		m.lines = append(m.lines, "run failed: "+payload.Error)
	}
}

func max(a, b int) int {
	if a > b {
		return a
	}
	return b
}
