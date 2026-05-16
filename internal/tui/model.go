package tui

import (
	"context"
	"errors"
	"fmt"
	"strings"
	"time"

	"charm.land/bubbles/v2/textarea"
	tea "charm.land/bubbletea/v2"
	zone "github.com/lrstanley/bubblezone/v2"
	"github.com/pandelisz/gode/internal/godex"
	"github.com/pandelisz/gode/internal/godex/codexauth"
	"github.com/pandelisz/gode/internal/godex/eventbus"
	"github.com/pandelisz/gode/internal/tui/attachments"
	"github.com/pandelisz/gode/internal/tui/components"
	"github.com/pandelisz/gode/internal/tui/dialogs"
	"github.com/pandelisz/gode/internal/tui/eventadapter"
	tuiremote "github.com/pandelisz/gode/internal/tui/remote"
	"github.com/pandelisz/gode/internal/tui/selection"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

const maxErrorLogEntries = 200
const wheelScrollLines = 3

type Model struct {
	app                     *godex.App
	zones                   *zone.Manager
	eventCancel             context.CancelFunc
	eventCh                 <-chan eventbus.Event
	transcript              components.TranscriptCache
	input                   textarea.Model
	messages                []viewmodel.Message
	messageKeys             map[string]string
	nextID                  int
	width                   int
	height                  int
	scrollOffset            int
	followTail              bool
	running                 bool
	hoveredID               string
	status                  string
	settings                dialogs.Settings
	remote                  *tuiremote.Controller
	remoteOpen              bool
	remoteState             tuiremote.State
	commands                dialogs.Commands
	sessions                dialogs.Sessions
	completions             dialogs.Commands
	completionMode          string
	permissions             dialogs.Permissions
	quitConfirmOpen         bool
	stopConfirmOpen         bool
	runCancel               context.CancelFunc
	currentRunID            string
	currentSessionID        string
	currentSession          string
	attachments             []attachments.Attachment
	codexLogin              func(context.Context, string) (codexauth.Tokens, string, error)
	imagePaste              func(context.Context, string) (attachments.Attachment, error)
	errorLog                []viewmodel.ErrorLogEntry
	showErrorLog            bool
	reasoningSummary        string
	contextLeft             string
	slashSelected           int
	goalSummary             string
	queuedPrompts           []pendingPrompt
	transcriptSelection     selection.Range
	transcriptMouseDown     bool
	transcriptLineRefs      []selection.TranscriptLineRef
	composerSelection       selection.OffsetRange
	composerMouseDown       bool
	clipboardWrite          selection.ClipboardWriter
	copyNoticeUntil         time.Time
	selectionCaptureEnabled bool
	captureDisabledUntil    time.Time
	captureRestoreSeq       int

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
		app:                     app,
		width:                   100,
		height:                  30,
		zones:                   zones,
		transcript:              components.NewTranscriptCache(),
		input:                   input,
		followTail:              true,
		status:                  "ready",
		codexLogin:              codexauth.LoginBrowser,
		imagePaste:              attachments.PasteImageFromClipboard,
		contextLeft:             defaultContextLeft(app),
		transcriptLineDirty:     true,
		messageKeys:             map[string]string{},
		clipboardWrite:          selection.SystemClipboardWriter,
		selectionCaptureEnabled: true,
	}
	if app != nil && app.Bus != nil {
		ctx, cancel := context.WithCancel(context.Background())
		model.eventCancel = cancel
		model.eventCh = app.Bus.Subscribe(ctx, eventbus.Filter{})
	}
	return model
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
		if m.stopConfirmOpen {
			return m.updateStopConfirm(msg)
		}
		if m.quitConfirmOpen {
			return m.updateQuitConfirm(msg)
		}
		if m.settings.Open {
			return m.updateSettings(msg)
		}
		if m.remoteOpen {
			return m.updateRemotePanel(msg)
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
		if m.handleSelectionKey(msg) {
			return m, nil
		}
		switch msg.String() {
		case "ctrl+c", "esc":
			if m.running {
				m.stopConfirmOpen = true
				m.status = "confirm stop"
				m.input.Blur()
				return m, nil
			}
			m.quitConfirmOpen = true
			m.status = "confirm quit"
			m.input.Blur()
			return m, nil
		case "ctrl+p":
			m.openSettings()
			return m, nil
		case "shift+tab":
			return m, m.togglePermissionMode(false)
		case "ctrl+s":
			m.openSessions()
			return m, nil
		case "ctrl+v", "ctrl+alt+v":
			return m, m.pasteImageFromClipboard()
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
			if prompt == "" && len(m.attachments) == 0 {
				return m, nil
			}
			if m.running {
				pending, err := m.preparePrompt(prompt)
				if err != nil {
					m.addMessage(viewmodel.RoleError, "attachments", err.Error())
					m.status = "attachment failed - ctrl+l errors"
					return m, nil
				}
				return m, m.steerPreparedPrompt(pending)
			}
			if handled, cmd := m.handleGoalInput(prompt); handled {
				m.input.Reset()
				return m, cmd
			}
			if handled, cmd := m.handleRemoteInput(prompt); handled {
				return m, cmd
			}
			pending, err := m.preparePrompt(prompt)
			if err != nil {
				m.addMessage(viewmodel.RoleError, "attachments", err.Error())
				m.status = "attachment failed - ctrl+l errors"
				return m, nil
			}
			return m, m.submitPreparedPrompt(pending)
		}
		if msg.String() == "tab" {
			if m.running {
				prompt := strings.TrimSpace(m.input.Value())
				if prompt == "" && len(m.attachments) == 0 {
					return m, nil
				}
				pending, err := m.preparePrompt(prompt)
				if err != nil {
					m.addMessage(viewmodel.RoleError, "attachments", err.Error())
					m.status = "attachment failed - ctrl+l errors"
					return m, nil
				}
				return m, m.queuePreparedPrompt(pending)
			}
			if m.openCompletionForCurrentToken() {
				return m, nil
			}
		}
		if msg.Text == "@" || msg.Text == "$" {
			m.input.InsertString(msg.Text)
			if m.openCompletionForCurrentToken() {
				return m, nil
			}
			return m, nil
		}
	case tea.PasteMsg:
		if attachment, ok := m.pastedImagePath(msg.Content); ok {
			m.attachImage(attachment)
			return m, nil
		}
	case imagePasteDoneMsg:
		if msg.Err != nil {
			m.failImagePaste(msg.Err)
			return m, nil
		}
		m.attachImage(msg.Attachment)
		return m, nil
	case tea.MouseWheelMsg:
		return m, m.handleWheel(msg)
	case tea.MouseMotionMsg:
		if m.updateComposerSelectionDrag(msg) {
			return m, nil
		}
		if m.updateTranscriptSelectionDrag(msg) {
			return m, nil
		}
		m.updateHover(msg)
		return m, nil
	case tea.MouseReleaseMsg:
		if m.finishComposerSelection(msg) {
			return m, nil
		}
		if m.finishTranscriptSelection(msg) {
			return m, nil
		}
	case tea.MouseClickMsg:
		if m.settings.Open {
			return m.updateSettingsMouse(msg)
		}
		if m.remoteOpen {
			return m, nil
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
		if m.startComposerSelection(msg) {
			return m, nil
		}
		if m.startTranscriptSelection(msg) {
			return m, nil
		}
		m.updateHover(msg)
	case eventMsg:
		m.capturePermissionRequest(msg.Event)
		m.applyGoalEvent(msg.Event)
		m.applyEvent(eventadapter.Apply(msg.Event))
		return m, m.waitForEvent()
	case runDoneMsg:
		m.running = false
		m.stopConfirmOpen = false
		m.currentRunID = ""
		m.runCancel = nil
		if msg.Result.SessionID != "" {
			m.setCurrentSession(msg.Result.SessionID)
		}
		if msg.Err != nil {
			if errors.Is(msg.Err, context.Canceled) {
				m.status = "run stopped"
				return m, nil
			}
			if !m.hasRecentError(msg.Err.Error()) {
				m.addMessage(viewmodel.RoleError, "", msg.Err.Error())
			}
			m.status = "run failed - ctrl+l errors"
		} else {
			m.status = "ready"
			if cmd := m.submitNextQueuedPrompt(); cmd != nil {
				return m, cmd
			}
		}
		return m, nil
	case steerDoneMsg:
		if msg.Err != nil {
			m.status = "steer failed - ctrl+l errors"
			m.addMessage(viewmodel.RoleError, "steer", msg.Err.Error())
		} else {
			m.status = "steer queued for active run"
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
	case remoteStateMsg:
		m.remoteState = msg.State
		if msg.Err != nil {
			m.status = "remote failed"
		} else if msg.State.Running {
			m.status = "remote running"
		} else {
			m.status = "remote stopped"
		}
		return m, nil
	case mouseCaptureRestoreMsg:
		m.restoreMouseCapture(msg)
		return m, nil
	}

	var cmd tea.Cmd
	m.input, cmd = m.input.Update(msg)
	m.clampSlashSelection()
	return m, cmd
}

func (m *Model) applyEvent(update eventadapter.Update) {
	for _, message := range update.Messages {
		m.addOrUpdateMessage(message)
	}
	if update.AssistantDelta != "" {
		m.appendAssistantDelta(update.AssistantDelta, update.AssistantPhase)
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

func (m Model) visibleReasoningSummary() string {
	if !m.running {
		return ""
	}
	return m.reasoningSummary
}

func (m *Model) addOrUpdateMessage(message eventadapter.Message) {
	if strings.TrimSpace(message.Key) == "" {
		m.addMessage(message.Role, message.Title, message.Body)
		return
	}
	if m.messageKeys == nil {
		m.messageKeys = map[string]string{}
	}
	if id := m.messageKeys[message.Key]; id != "" {
		for i := range m.messages {
			if m.messages[i].ID != id {
				continue
			}
			if message.Role == viewmodel.RoleError {
				m.addErrorLog(message.Title, message.Body)
				message.Body = summarizeTimelineError(message.Body)
			} else if message.Role == viewmodel.RoleTool && isFailedToolBody(message.Body) {
				m.addErrorLog(message.Title, failedToolError(message.Body))
			}
			m.messages[i].Role = message.Role
			m.messages[i].Title = message.Title
			m.messages[i].Body = message.Body
			m.markTranscriptLinesDirty()
			if m.followTail {
				m.scrollOffset = 0
			} else {
				m.clampScroll()
			}
			m.reconcileTranscriptSelection()
			return
		}
		delete(m.messageKeys, message.Key)
	}
	m.addMessage(message.Role, message.Title, message.Body)
	if len(m.messages) > 0 {
		m.messageKeys[message.Key] = m.messages[len(m.messages)-1].ID
	}
}

func (m *Model) addMessage(role viewmodel.Role, title string, body string) {
	if role == viewmodel.RoleError {
		m.addErrorLog(title, body)
		body = summarizeTimelineError(body)
	} else if role == viewmodel.RoleTool && isFailedToolBody(body) {
		m.addErrorLog(title, failedToolError(body))
	}
	m.nextID++
	m.messages = append(m.messages, viewmodel.Message{
		ID:    fmt.Sprintf("m%d", m.nextID),
		Role:  role,
		Title: title,
		Body:  body,
	})
	m.markTranscriptLinesDirty()
	if m.followTail {
		m.scrollOffset = 0
		m.reconcileTranscriptSelection()
		return
	}
	m.clampScroll()
	m.reconcileTranscriptSelection()
}

func (m *Model) pruneMessageKeys() {
	if len(m.messageKeys) == 0 {
		return
	}
	seen := make(map[string]struct{}, len(m.messages))
	for _, message := range m.messages {
		seen[message.ID] = struct{}{}
	}
	for key, id := range m.messageKeys {
		if _, ok := seen[id]; !ok {
			delete(m.messageKeys, key)
		}
	}
}

func isFailedToolBody(body string) bool {
	return strings.HasPrefix(strings.TrimSpace(body), "failed:")
}

func failedToolError(body string) string {
	return strings.TrimSpace(strings.TrimPrefix(strings.TrimSpace(body), "failed:"))
}

func (m *Model) appendAssistantDelta(text string, phase ...string) {
	title := assistantPhaseTitle(firstNonEmpty(phase...))
	if len(m.messages) > 0 {
		last := &m.messages[len(m.messages)-1]
		if last.Role == viewmodel.RoleAssistant && last.Title == title {
			last.Body += text
			m.markTranscriptLinesDirty()
			if m.followTail {
				m.scrollOffset = 0
			}
			m.reconcileTranscriptSelection()
			return
		}
	}
	m.addMessage(viewmodel.RoleAssistant, title, text)
}

func assistantPhaseTitle(phase string) string {
	switch strings.TrimSpace(phase) {
	case "commentary":
		return "commentary"
	default:
		return ""
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
