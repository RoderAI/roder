package tui

import (
	"context"

	tea "charm.land/bubbletea/v2"
	tuiremote "github.com/pandelisz/gode/internal/tui/remote"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

type remoteStateMsg struct {
	State tuiremote.State
	Err   error
}

type remoteCopyMsg struct {
	Status string
}

func (m *Model) ensureRemoteController() *tuiremote.Controller {
	if m.remote == nil {
		m.remote = tuiremote.NewController(m.app)
	}
	return m.remote
}

func (m *Model) handleRemoteInput(prompt string) (bool, tea.Cmd) {
	if prompt != "/remote" {
		return false, nil
	}
	m.input.Reset()
	m.openRemotePanel()
	return true, nil
}

func (m *Model) openRemotePanel() {
	controller := m.ensureRemoteController()
	m.remoteState = controller.Snapshot()
	m.remoteOpen = true
	m.status = "remote control"
	m.input.Blur()
}

func (m Model) closeRemotePanel(status string) tea.Cmd {
	m.remoteOpen = false
	m.status = status
	return m.input.Focus()
}

func (m Model) updateRemotePanel(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	switch msg.String() {
	case "ctrl+c":
		return m, tea.Quit
	case "esc", "escape", "ctrl+[", "ctrl+p":
		return m, m.closeRemotePanel("ready")
	case "enter", " ":
		if m.remoteState.Running {
			return m, m.stopRemoteServer()
		}
		return m, m.startRemoteServer()
	case "r":
		return m, m.regenerateRemoteServer()
	case "u":
		return m, m.copyRemoteURL()
	case "h":
		return m, m.copyRemoteAuthHeader()
	}
	return m, nil
}

func (m Model) copyRemoteURL() tea.Cmd {
	if len(m.remoteState.URLs) == 0 {
		m.status = "remote url unavailable"
		return nil
	}
	return m.copyRemoteText(m.remoteState.URLs[0], "Copied remote URL")
}

func (m Model) copyRemoteAuthHeader() tea.Cmd {
	if m.remoteState.AuthHeader == "" {
		m.status = "remote auth header unavailable"
		return nil
	}
	return m.copyRemoteText(m.remoteState.AuthHeader, "Copied remote auth header")
}

func (m Model) copyRemoteText(text string, status string) tea.Cmd {
	writer := m.clipboardWrite
	if writer == nil {
		return nil
	}
	return func() tea.Msg {
		if err := writer(text); err != nil {
			return remoteCopyMsg{Status: "clipboard failed - " + truncateStatus(err.Error(), 120)}
		}
		return remoteCopyMsg{Status: status}
	}
}

func (m Model) startRemoteServer() tea.Cmd {
	controller := m.ensureRemoteController()
	return func() tea.Msg {
		state, err := controller.Start(context.Background())
		return remoteStateMsg{State: state, Err: err}
	}
}

func (m Model) stopRemoteServer() tea.Cmd {
	controller := m.ensureRemoteController()
	return func() tea.Msg {
		state, err := controller.Stop(context.Background())
		return remoteStateMsg{State: state, Err: err}
	}
}

func (m Model) regenerateRemoteServer() tea.Cmd {
	controller := m.ensureRemoteController()
	return func() tea.Msg {
		state, err := controller.Regenerate(context.Background())
		return remoteStateMsg{State: state, Err: err}
	}
}

func (m Model) remoteViewModel() *viewmodel.RemoteDialog {
	if !m.remoteOpen {
		return nil
	}
	state := m.remoteState
	return &viewmodel.RemoteDialog{
		Title:            "Remote Control",
		Running:          state.Running,
		URLs:             state.URLs,
		TokenPreview:     state.TokenPreview,
		QR:               state.QR,
		AuthHeaderHint:   state.AuthHeaderHint,
		SubprotocolHint:  state.SubprotocolHint,
		ConnectedClients: state.ConnectedClients,
		Warning:          tuiremote.SecurityWarning(state),
		Error:            state.Error,
		Help:             "enter start/stop  r regenerate  u copy url  h copy auth  esc close",
	}
}
