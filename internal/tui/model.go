package tui

import (
	"context"
	"fmt"
	"strings"

	"charm.land/bubbles/v2/textarea"
	tea "charm.land/bubbletea/v2"
	"charm.land/lipgloss/v2"
	zone "github.com/lrstanley/bubblezone/v2"
	"github.com/pandelisz/gode/internal/godex"
	"github.com/pandelisz/gode/internal/godex/agent"
	"github.com/pandelisz/gode/internal/godex/codexauth"
	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/tui/attachments"
	"github.com/pandelisz/gode/internal/tui/components"
	"github.com/pandelisz/gode/internal/tui/dialogs"
	"github.com/pandelisz/gode/internal/tui/eventadapter"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

const maxTranscriptMessages = 500
const maxErrorLogEntries = 200
const wheelScrollLines = 3

type eventMsg struct {
	Event eventbus.Event
}

type runDoneMsg struct {
	Result agent.RunResult
	Err    error
}

type codexAuthDoneMsg struct {
	AccountID string
	Err       error
}

type skillsInstallDoneMsg struct {
	Installed int
	Source    string
	Output    string
	Err       error
}

type Model struct {
	app              *godex.App
	zones            *zone.Manager
	eventCancel      context.CancelFunc
	eventCh          <-chan eventbus.Event
	transcript       components.TranscriptCache
	input            textarea.Model
	messages         []viewmodel.Message
	nextID           int
	width            int
	height           int
	scrollOffset     int
	followTail       bool
	running          bool
	hoveredID        string
	status           string
	settings         dialogs.Settings
	commands         dialogs.Commands
	sessions         dialogs.Sessions
	completions      dialogs.Commands
	completionMode   string
	permissions      dialogs.Permissions
	currentSessionID string
	currentSession   string
	attachments      []attachments.Attachment
	codexLogin       func(context.Context, string) (codexauth.Tokens, string, error)
	errorLog         []viewmodel.ErrorLogEntry
	showErrorLog     bool
	reasoningSummary string
	contextLeft      string
	slashSelected    int
	goalSummary      string

	transcriptLineWidth int
	transcriptLineTotal int
	transcriptLineDirty bool
}

func New(app *godex.App) Model {
	zones := zone.New()
	zones.SetEnabled(true)
	input := textarea.New()
	input.Placeholder = "Ask gode to work on this repo"
	input.Prompt = ""
	input.ShowLineNumbers = false
	input.DynamicHeight = true
	input.MinHeight = 1
	input.MaxHeight = 6
	input.SetWidth(80)
	applyComposerStyles(&input)
	input.Focus()
	model := Model{
		app:                 app,
		zones:               zones,
		transcript:          components.NewTranscriptCache(),
		input:               input,
		followTail:          true,
		status:              "ready",
		codexLogin:          codexauth.LoginBrowser,
		contextLeft:         defaultContextLeft(app),
		transcriptLineDirty: true,
	}
	if app != nil && app.Bus != nil {
		ctx, cancel := context.WithCancel(context.Background())
		model.eventCancel = cancel
		model.eventCh = app.Bus.Subscribe(ctx, eventbus.Filter{})
	}
	return model
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
		if m.settings.Open {
			return m.updateSettings(msg)
		}
		if m.permissions.Open {
			return m.updatePermissions(msg)
		}
		if m.completions.Open {
			return m.updateCompletions(msg)
		}
		if m.commands.Open {
			return m.updateCommands(msg)
		}
		if m.sessions.Open {
			return m.updateSessions(msg)
		}
		if m.inlineSlashMenuOpen() {
			switch msg.String() {
			case "down", "j":
				m.moveSlashSelection(1)
				return m, nil
			case "up", "k":
				m.moveSlashSelection(-1)
				return m, nil
			case "tab", "enter":
				return m.acceptSlashSelection()
			case "esc", "escape", "ctrl+[":
				m.input.Reset()
				m.slashSelected = 0
				m.status = "ready"
				return m, nil
			}
		}
		switch msg.String() {
		case "ctrl+c", "esc":
			m.cancelEvents()
			return m, tea.Quit
		case "ctrl+p":
			m.openSettings()
			return m, nil
		case "ctrl+s":
			m.openSessions()
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
			if handled, cmd := m.handleGoalInput(prompt); handled {
				m.input.Reset()
				return m, cmd
			}
			runPrompt := prompt
			if len(m.attachments) > 0 {
				withAttachments, err := m.promptWithAttachments(prompt)
				if err != nil {
					m.addMessage(viewmodel.RoleError, "attachments", err.Error())
					m.status = "attachment failed - ctrl+l errors"
					return m, nil
				}
				runPrompt = withAttachments
			}
			m.addMessage(viewmodel.RoleUser, "", prompt)
			m.reasoningSummary = ""
			m.attachments = nil
			m.input.Reset()
			m.running = true
			m.status = "waiting for model"
			return m, m.runPrompt(runPrompt)
		}
		if msg.String() == "tab" && m.openCompletionForCurrentToken() {
			return m, nil
		}
		if msg.Text == "@" || msg.Text == "$" {
			m.input.InsertString(msg.Text)
			if m.openCompletionForCurrentToken() {
				return m, nil
			}
			return m, nil
		}
	case tea.MouseWheelMsg:
		m.handleWheel(msg)
		return m, nil
	case tea.MouseMotionMsg:
		m.updateHover(msg)
		return m, nil
	case tea.MouseClickMsg:
		if m.settings.Open {
			return m.updateSettingsMouse(msg)
		}
		if m.permissions.Open {
			return m.updatePermissionsMouse(msg)
		}
		if m.completions.Open {
			return m.updateCompletionsMouse(msg)
		}
		if m.commands.Open {
			return m.updateCommandsMouse(msg)
		}
		if m.sessions.Open {
			return m.updateSessionsMouse(msg)
		}
		if m.inlineSlashMenuOpen() {
			return m.updateSlashMenuMouse(msg)
		}
		m.updateHover(msg)
	case eventMsg:
		m.capturePermissionRequest(msg.Event)
		m.applyGoalEvent(msg.Event)
		m.applyEvent(eventadapter.Apply(msg.Event))
		return m, m.waitForEvent()
	case runDoneMsg:
		m.running = false
		if msg.Result.SessionID != "" {
			m.setCurrentSession(msg.Result.SessionID)
		}
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
	case skillsInstallDoneMsg:
		if msg.Err != nil {
			m.settings.InstallPrompt.Installing = false
			m.settings.Err = truncateStatus(firstNonEmpty(msg.Output, msg.Err.Error()), 160)
			m.status = "skill install failed"
			m.addMessage(viewmodel.RoleSystem, "skill install", skillInstallTranscript(msg))
			return m, nil
		}
		if m.settings.Open {
			m.refreshSettingsSkills()
			m.settings.OpenSkills()
		}
		m.addMessage(viewmodel.RoleSystem, "skill install", skillInstallTranscript(msg))
		m.status = fmt.Sprintf("installed %d skills", msg.Installed)
		return m, nil
	}

	var cmd tea.Cmd
	m.input, cmd = m.input.Update(msg)
	m.clampSlashSelection()
	return m, cmd
}

func (m *Model) applyEvent(update eventadapter.Update) {
	for _, message := range update.Messages {
		m.addMessage(message.Role, message.Title, message.Body)
	}
	if update.AssistantDelta != "" {
		m.appendAssistantDelta(update.AssistantDelta)
	}
	if update.ReasoningDelta != "" {
		m.reasoningSummary += update.ReasoningDelta
	}
	if update.HasReasoningSummary {
		m.reasoningSummary = update.ReasoningSummary
	}
	if update.HasContextTokens {
		m.contextLeft = formatContextLeft(update.ContextUsedPercent)
	}
	if update.Running != nil {
		m.running = *update.Running
	}
	if update.HasStatus {
		m.status = update.Status
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
