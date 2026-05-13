package tui

import (
	"context"
	"fmt"
	"strings"
	"time"

	"charm.land/bubbles/v2/textarea"
	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"
	zone "github.com/lrstanley/bubblezone/v2"
	"github.com/pandelisz/gode/internal/godex"
	"github.com/pandelisz/gode/internal/godex/codexauth"
	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/tui/components"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

const maxTranscriptMessages = 500
const maxErrorLogEntries = 200
const wheelScrollLines = 3

type eventMsg struct {
	Event eventbus.Event
}

type runDoneMsg struct {
	Err error
}

type codexAuthDoneMsg struct {
	AccountID string
	Err       error
}

type Model struct {
	app          *godex.App
	zones        *zone.Manager
	transcript   components.TranscriptCache
	input        textarea.Model
	messages     []viewmodel.Message
	nextID       int
	width        int
	height       int
	scrollOffset int
	followTail   bool
	running      bool
	hoveredID    string
	status       string
	settings     settingsDialog
	codexLogin   func(context.Context, string) (codexauth.Tokens, string, error)
	errorLog     []viewmodel.ErrorLogEntry
	showErrorLog bool

	transcriptLineWidth int
	transcriptLineTotal int
	transcriptLineDirty bool
}

func New(app *godex.App) Model {
	zones := zone.New()
	zones.SetEnabled(true)
	input := textarea.New()
	input.Placeholder = "Ask gode to work on this repo"
	input.Prompt = "> "
	input.ShowLineNumbers = false
	input.DynamicHeight = true
	input.MinHeight = 1
	input.MaxHeight = 6
	input.SetWidth(80)
	applyComposerStyles(&input)
	input.Focus()
	return Model{
		app:                 app,
		zones:               zones,
		transcript:          components.NewTranscriptCache(),
		input:               input,
		followTail:          true,
		status:              "ready",
		codexLogin:          codexauth.LoginBrowser,
		transcriptLineDirty: true,
	}
}

func applyComposerStyles(input *textarea.Model) {
	styles := input.Styles()
	styles.Focused.CursorLine = lipgloss.NewStyle()
	styles.Focused.Prompt = lipgloss.NewStyle().Foreground(lipgloss.Color("252"))
	styles.Focused.Placeholder = lipgloss.NewStyle().Foreground(lipgloss.Color("244"))
	styles.Focused.Text = lipgloss.NewStyle().Foreground(lipgloss.Color("252"))
	styles.Blurred.CursorLine = lipgloss.NewStyle()
	styles.Blurred.Prompt = lipgloss.NewStyle().Foreground(lipgloss.Color("246"))
	styles.Blurred.Placeholder = lipgloss.NewStyle().Foreground(lipgloss.Color("240"))
	styles.Blurred.Text = lipgloss.NewStyle().Foreground(lipgloss.Color("246"))
	input.SetStyles(styles)
}

func (m Model) Init() tea.Cmd {
	return m.waitForEvent()
}

func (m Model) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.WindowSizeMsg:
		oldWrapWidth := m.transcriptWrapWidth()
		m.width = msg.Width
		m.height = msg.Height
		m.input.SetWidth(max(20, msg.Width-4))
		if m.transcriptWrapWidth() != oldWrapWidth {
			m.markTranscriptLinesDirty()
		}
		m.resizeSettings()
		m.clampScroll()
		return m, nil
	case tea.KeyPressMsg:
		if m.settings.open {
			return m.updateSettings(msg)
		}
		switch msg.String() {
		case "ctrl+c", "esc":
			return m, tea.Quit
		case "ctrl+p":
			m.openSettings()
			return m, nil
		case "ctrl+l":
			m.showErrorLog = !m.showErrorLog
			if m.showErrorLog {
				m.status = "error log open"
			} else {
				m.status = "ready"
			}
			m.clampScroll()
			return m, nil
		case "pgup":
			m.scrollBy(max(4, m.visibleTranscriptLines()-1))
			return m, nil
		case "pgdown":
			m.scrollBy(-max(4, m.visibleTranscriptLines()-1))
			return m, nil
		case "home":
			m.scrollToOldest()
			return m, nil
		case "end":
			m.follow()
			return m, nil
		case "enter":
			prompt := strings.TrimSpace(m.input.Value())
			if prompt == "" {
				return m, nil
			}
			m.addMessage(viewmodel.RoleUser, "", prompt)
			m.input.Reset()
			m.running = true
			m.status = "waiting for model"
			return m, m.runPrompt(prompt)
		}
	case tea.MouseWheelMsg:
		m.handleWheel(msg)
		return m, nil
	case tea.MouseMotionMsg:
		m.updateHover(msg)
		return m, nil
	case tea.MouseClickMsg:
		if m.settings.open {
			return m.updateSettingsMouse(msg)
		}
		m.updateHover(msg)
	case eventMsg:
		m.appendEvent(msg.Event)
		return m, m.waitForEvent()
	case runDoneMsg:
		m.running = false
		if msg.Err != nil {
			if !m.hasRecentError(msg.Err.Error()) {
				m.addMessage(viewmodel.RoleError, "", msg.Err.Error())
			}
			m.status = "run failed - ctrl+l errors"
		} else {
			m.status = "ready"
		}
		return m, nil
	case codexAuthDoneMsg:
		if msg.Err != nil {
			m.status = "codex sign-in failed"
			m.addMessage(viewmodel.RoleError, "codex sign-in", msg.Err.Error())
			return m, nil
		}
		if msg.AccountID != "" {
			m.status = "signed in to codex: " + msg.AccountID
		} else {
			m.status = "signed in to codex"
		}
		return m, nil
	}

	var cmd tea.Cmd
	m.input, cmd = m.input.Update(msg)
	return m, cmd
}

func (m Model) View() tea.View {
	vm := viewmodel.Model{
		Width:        m.width,
		Height:       m.height,
		Messages:     m.messages,
		Input:        m.input.View(),
		InputHeight:  m.input.Height(),
		ScrollOffset: m.scrollOffset,
		FollowTail:   m.followTail,
		Running:      m.running,
		HoveredID:    m.hoveredID,
		Status:       m.status,
		Settings:     m.settings.viewModel(),
		ErrorLog:     m.errorLog,
		ShowErrorLog: m.showErrorLog,
	}
	if m.app != nil {
		vm.Provider = godex.DisplayProvider(m.app.Config)
		vm.Model = m.app.Config.Model
		vm.Reasoning = m.app.Config.Reasoning
	}
	view := tea.NewView(m.zones.Scan(components.RenderWithCache(vm, m.zones, &m.transcript)))
	view.AltScreen = true
	view.MouseMode = tea.MouseModeAllMotion
	view.WindowTitle = "gode"
	return view
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
			m.appendAssistantDelta(payload.Text)
		}
	case eventbus.KindAssistantCompleted:
		m.status = "assistant completed"
	case eventbus.KindToolRequested:
		var payload struct {
			Tool string `json:"tool"`
		}
		_ = ev.DecodePayload(&payload)
		m.addMessage(viewmodel.RoleTool, payload.Tool, "requested")
		m.status = "tool requested: " + payload.Tool
	case eventbus.KindToolStarted:
		var payload struct {
			Tool string `json:"tool"`
		}
		_ = ev.DecodePayload(&payload)
		m.status = "tool running: " + payload.Tool
	case eventbus.KindToolCompleted:
		var payload struct {
			Tool string `json:"tool"`
			Text string `json:"text"`
		}
		_ = ev.DecodePayload(&payload)
		m.addMessage(viewmodel.RoleTool, payload.Tool, truncate(payload.Text, 1600))
		m.status = "tool completed: " + payload.Tool
	case eventbus.KindToolFailed:
		var payload struct {
			Tool  string `json:"tool"`
			Error string `json:"error"`
		}
		_ = ev.DecodePayload(&payload)
		m.addMessage(viewmodel.RoleError, payload.Tool, payload.Error)
		m.status = "tool failed: " + payload.Tool + " - ctrl+l errors"
	case eventbus.KindPermissionRequested:
		var payload struct {
			Tool string `json:"tool"`
		}
		_ = ev.DecodePayload(&payload)
		m.addMessage(viewmodel.RoleSystem, "permission", payload.Tool)
		m.status = "permission requested"
	case eventbus.KindRunCompleted:
		m.running = false
		m.status = "run completed"
	case eventbus.KindRunFailed:
		var payload struct {
			Error string `json:"error"`
		}
		_ = ev.DecodePayload(&payload)
		m.running = false
		m.addMessage(viewmodel.RoleError, "", payload.Error)
		m.status = "run failed - ctrl+l errors"
	}
}

func (m *Model) addMessage(role viewmodel.Role, title string, body string) {
	if role == viewmodel.RoleError {
		m.addErrorLog(title, body)
		body = summarizeTimelineError(body)
	}
	m.nextID++
	m.messages = append(m.messages, viewmodel.Message{
		ID:    fmt.Sprintf("m%d", m.nextID),
		Role:  role,
		Title: title,
		Body:  body,
	})
	if overflow := len(m.messages) - maxTranscriptMessages; overflow > 0 {
		m.messages = m.messages[overflow:]
		m.scrollOffset = max(0, m.scrollOffset-overflow)
		m.transcript.Prune(m.messages)
		m.markTranscriptLinesDirty()
	}
	m.markTranscriptLinesDirty()
	if m.followTail {
		m.scrollOffset = 0
		return
	}
	m.clampScroll()
}

func (m *Model) addErrorLog(source string, message string) {
	message = strings.TrimSpace(message)
	if message == "" {
		return
	}
	source = strings.TrimSpace(source)
	if source == "" {
		source = "run"
	}
	m.errorLog = append(m.errorLog, viewmodel.ErrorLogEntry{
		ID:      fmt.Sprintf("e%d", len(m.errorLog)+1),
		Time:    time.Now().Format("15:04:05"),
		Source:  source,
		Message: message,
	})
	if overflow := len(m.errorLog) - maxErrorLogEntries; overflow > 0 {
		m.errorLog = m.errorLog[overflow:]
	}
}

func (m Model) hasRecentError(message string) bool {
	message = strings.TrimSpace(message)
	if message == "" {
		return false
	}
	for i := len(m.errorLog) - 1; i >= 0 && i >= len(m.errorLog)-3; i-- {
		entry := m.errorLog[i]
		if strings.TrimSpace(entry.Message) == message {
			return true
		}
	}
	return false
}

func summarizeTimelineError(message string) string {
	message = strings.TrimSpace(message)
	if message == "" {
		return ""
	}
	lines := nonEmptyLines(message)
	first := lines[0]
	if status := valueLine(lines, "status: "); status != "" {
		if first == "OpenAI stream request failed" {
			if apiMessage := valueLine(lines, "error_message: "); apiMessage != "" {
				return truncateSingleLine(first+": "+status+" - "+apiMessage+" - ctrl+l for details", 180)
			}
			return truncateSingleLine(first+": "+status+" - ctrl+l for details", 180)
		}
		return truncateSingleLine(status+" - ctrl+l for details", 180)
	}
	if len(lines) > 1 {
		return truncateSingleLine(first+" - ctrl+l for details", 180)
	}
	return truncateSingleLine(first, 180)
}

func nonEmptyLines(message string) []string {
	lines := []string{}
	for _, line := range strings.Split(message, "\n") {
		line = strings.TrimSpace(line)
		if line != "" {
			lines = append(lines, line)
		}
	}
	if len(lines) == 0 {
		return []string{""}
	}
	return lines
}

func valueLine(lines []string, prefix string) string {
	for _, line := range lines {
		if strings.HasPrefix(line, prefix) {
			return strings.TrimSpace(strings.TrimPrefix(line, prefix))
		}
	}
	return ""
}

func truncateSingleLine(text string, limit int) string {
	text = strings.Join(strings.Fields(text), " ")
	if len(text) <= limit {
		return text
	}
	return text[:limit] + "..."
}

func (m *Model) appendAssistantDelta(text string) {
	if len(m.messages) > 0 {
		last := &m.messages[len(m.messages)-1]
		if last.Role == viewmodel.RoleAssistant {
			last.Body += text
			m.markTranscriptLinesDirty()
			if m.followTail {
				m.scrollOffset = 0
			}
			return
		}
	}
	m.addMessage(viewmodel.RoleAssistant, "", text)
}

func (m *Model) handleWheel(msg tea.MouseWheelMsg) {
	if transcript := m.zones.Get(viewmodel.TranscriptZoneID); transcript != nil && !transcript.InBounds(msg) {
		return
	}
	mouse := msg.Mouse()
	switch mouse.Button {
	case tea.MouseWheelUp:
		m.scrollBy(wheelScrollLines)
	case tea.MouseWheelDown:
		m.scrollBy(-wheelScrollLines)
	}
}

func (m *Model) updateHover(msg tea.MouseMsg) {
	for _, item := range m.messages {
		z := m.zones.Get(viewmodel.MessageZoneID(item.ID))
		if z != nil && z.InBounds(msg) {
			m.hoveredID = item.ID
			return
		}
	}
	m.hoveredID = ""
}

func truncate(text string, limit int) string {
	if len(text) <= limit {
		return text
	}
	return text[:limit] + "\n... truncated in TUI; full result is in the event journal"
}

func max(a, b int) int {
	if a > b {
		return a
	}
	return b
}

func clamp(v int, low int, high int) int {
	if high < low {
		return low
	}
	if v < low {
		return low
	}
	if v > high {
		return high
	}
	return v
}
