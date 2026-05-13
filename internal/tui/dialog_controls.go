package tui

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"

	tea "charm.land/bubbletea/v2"
	"github.com/pandelisz/gode/internal/godex/eventbus"
	messagestore "github.com/pandelisz/gode/internal/godex/message"
	"github.com/pandelisz/gode/internal/tui/dialogs"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

func (m *Model) openCommands() {
	m.commands = dialogs.NewCommands(m.commandItems())
	m.input.Blur()
	m.status = "commands"
}

func (m *Model) openSessions() {
	m.sessions = dialogs.NewSessions(m.sessionItems())
	m.input.Blur()
	m.status = "sessions"
}

func (m Model) updateCommands(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	switch msg.String() {
	case "esc", "escape", "ctrl+[", "ctrl+p":
		m.commands = dialogs.Commands{}
		m.status = "ready"
		return m, m.input.Focus()
	case "down", "j":
		m.commands.Move(1)
		return m, nil
	case "up", "k":
		m.commands.Move(-1)
		return m, nil
	case "enter":
		return m.acceptCommandSelection()
	}
	return m, nil
}

func (m Model) updateCommandsMouse(msg tea.MouseClickMsg) (tea.Model, tea.Cmd) {
	for i, item := range m.commands.Items {
		z := m.zones.Get(viewmodel.DialogItemZoneID("commands", item.ID))
		if z != nil && z.InBounds(msg) {
			m.commands.Selected = i
			return m.acceptCommandSelection()
		}
	}
	return m, nil
}

func (m Model) acceptCommandSelection() (tea.Model, tea.Cmd) {
	item := m.commands.SelectedItem()
	if item.ID == "" {
		m.commands.Err = "no command selected"
		return m, nil
	}
	if strings.HasPrefix(item.ID, "mcp:") {
		m.input.SetValue("Use MCP prompt " + strings.TrimPrefix(item.ID, "mcp:") + " ")
	} else {
		m.input.SetValue("/" + item.ID + " ")
	}
	m.commands = dialogs.Commands{}
	m.status = "command inserted"
	return m, m.input.Focus()
}

func (m Model) updateSessions(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	switch msg.String() {
	case "esc", "escape", "ctrl+[", "ctrl+s":
		m.sessions = dialogs.Sessions{}
		m.status = "ready"
		return m, m.input.Focus()
	case "down", "j":
		m.sessions.Move(1)
		return m, nil
	case "up", "k":
		m.sessions.Move(-1)
		return m, nil
	case "enter":
		return m.acceptSessionSelection()
	}
	return m, nil
}

func (m Model) updateSessionsMouse(msg tea.MouseClickMsg) (tea.Model, tea.Cmd) {
	for i, item := range m.sessions.Items {
		z := m.zones.Get(viewmodel.DialogItemZoneID("sessions", item.ID))
		if z != nil && z.InBounds(msg) {
			m.sessions.Selected = i
			return m.acceptSessionSelection()
		}
	}
	return m, nil
}

func (m Model) acceptSessionSelection() (tea.Model, tea.Cmd) {
	item := m.sessions.SelectedItem()
	if item.ID == "" {
		m.sessions.Err = "no session selected"
		return m, nil
	}
	if err := m.loadSessionMessages(item.ID); err != nil {
		m.sessions.Err = err.Error()
		return m, nil
	}
	m.currentSessionID = item.ID
	m.sessions = dialogs.Sessions{}
	m.status = "session loaded"
	return m, m.input.Focus()
}

func (m Model) updatePermissions(msg tea.KeyPressMsg) (tea.Model, tea.Cmd) {
	switch msg.String() {
	case "down", "j":
		m.permissions.Move(1)
		return m, nil
	case "up", "k":
		m.permissions.Move(-1)
		return m, nil
	case "a", "enter":
		return m.respondPermission(true, false, "allowed")
	case "s":
		return m.respondPermission(true, true, "allowed for session")
	case "d", "esc", "escape", "ctrl+[":
		return m.respondPermission(false, false, "denied")
	}
	return m, nil
}

func (m Model) updatePermissionsMouse(msg tea.MouseClickMsg) (tea.Model, tea.Cmd) {
	for i, req := range m.permissions.Requests {
		z := m.zones.Get(viewmodel.DialogItemZoneID("permissions", req.ID))
		if z != nil && z.InBounds(msg) {
			m.permissions.Selected = i
			return m, nil
		}
	}
	return m, nil
}

func (m Model) respondPermission(approved bool, allowForSession bool, reason string) (tea.Model, tea.Cmd) {
	req := m.permissions.RemoveSelected()
	if req.CorrelationID == "" {
		m.permissions.Err = "permission correlation id missing"
		return m, nil
	}
	if m.app != nil && m.app.Bus != nil {
		m.app.Bus.Publish(context.Background(), eventbus.Event{
			Kind:          eventbus.KindPermissionResponded,
			Source:        eventbus.SourceTUI,
			SessionID:     req.SessionID,
			RunID:         req.RunID,
			CorrelationID: req.CorrelationID,
			Payload: map[string]any{
				"approved":          approved,
				"allow_for_session": allowForSession,
				"reason":            reason,
			},
		})
	}
	if !m.permissions.Open {
		m.status = "permission " + reason
		return m, m.input.Focus()
	}
	m.status = "permission " + reason
	return m, nil
}

func (m *Model) capturePermissionRequest(ev eventbus.Event) {
	if ev.Kind != eventbus.KindPermissionRequested {
		return
	}
	var payload struct {
		Tool        string         `json:"tool"`
		Action      string         `json:"action"`
		Path        string         `json:"path"`
		Description string         `json:"description"`
		Input       map[string]any `json:"input"`
	}
	_ = ev.DecodePayload(&payload)
	input := payload.Description
	if input == "" {
		input = payload.Path
	}
	if input == "" && payload.Input != nil {
		data, _ := json.Marshal(payload.Input)
		input = string(data)
	}
	req := dialogs.PermissionRequest{
		ID:            firstNonEmpty(ev.CorrelationID, ev.ID),
		CorrelationID: ev.CorrelationID,
		SessionID:     ev.SessionID,
		RunID:         ev.RunID,
		Tool:          payload.Tool,
		Action:        payload.Action,
		Input:         input,
	}
	if req.ID == "" {
		req.ID = fmt.Sprintf("permission-%d", len(m.permissions.Requests)+1)
	}
	m.permissions.Open = true
	m.permissions.Requests = append(m.permissions.Requests, req)
	m.permissions.Move(0)
	m.input.Blur()
	m.status = "permission requested"
}

func (m Model) commandsViewModel() *viewmodel.ListDialog {
	if !m.commands.Open {
		return nil
	}
	items := make([]viewmodel.ListDialogItem, 0, len(m.commands.Items))
	for _, item := range m.commands.Items {
		items = append(items, viewmodel.ListDialogItem{
			ID:          item.ID,
			Label:       item.Title,
			Description: item.Description,
			Value:       item.Source,
			Selected:    item.Selected,
		})
	}
	return &viewmodel.ListDialog{
		Kind:  "commands",
		Title: "Commands",
		Help:  "enter insert  esc close  up/down navigate",
		Items: items,
		Error: m.commands.Err,
	}
}

func (m Model) sessionsViewModel() *viewmodel.ListDialog {
	if !m.sessions.Open {
		return nil
	}
	items := make([]viewmodel.ListDialogItem, 0, len(m.sessions.Items))
	for _, item := range m.sessions.Items {
		items = append(items, viewmodel.ListDialogItem{
			ID:          item.ID,
			Label:       item.Title,
			Description: item.ID,
			Value:       item.Value(),
			Selected:    item.Selected,
		})
	}
	return &viewmodel.ListDialog{
		Kind:  "sessions",
		Title: "Sessions",
		Help:  "enter load  esc close  up/down navigate",
		Items: items,
		Error: m.sessions.Err,
	}
}

func (m Model) permissionsViewModel() *viewmodel.PermissionDialog {
	if !m.permissions.Open {
		return nil
	}
	requests := make([]viewmodel.PermissionDialogRequest, 0, len(m.permissions.Requests))
	for _, req := range m.permissions.Requests {
		requests = append(requests, viewmodel.PermissionDialogRequest{
			ID:       req.ID,
			Tool:     req.Tool,
			Action:   req.Action,
			Input:    req.Input,
			Selected: req.Selected,
		})
	}
	return &viewmodel.PermissionDialog{
		Title:    "Permission",
		Help:     "a allow  s allow session  d deny  up/down navigate",
		Requests: requests,
		Error:    m.permissions.Err,
	}
}

func (m Model) commandItems() []dialogs.CommandItem {
	if m.app == nil {
		return nil
	}
	items := []dialogs.CommandItem{}
	for _, command := range m.app.Commands() {
		items = append(items, dialogs.CommandItem{
			ID:          command.ID,
			Title:       "/" + command.ID,
			Description: firstLine(command.Prompt),
			Source:      command.Scope,
		})
	}
	if m.app.MCP != nil {
		for _, prompt := range m.app.MCP.Prompts() {
			id := "mcp:" + prompt.Server + ":" + prompt.Name
			items = append(items, dialogs.CommandItem{
				ID:          id,
				Title:       promptTitle(prompt.Title, prompt.Name),
				Description: prompt.Description,
				Source:      "mcp/" + prompt.Server,
			})
		}
	}
	return items
}

func (m Model) sessionItems() []dialogs.SessionItem {
	if m.app == nil || m.app.Sessions == nil {
		return nil
	}
	sessions, err := m.app.Sessions.List(context.Background())
	if err != nil {
		return nil
	}
	items := make([]dialogs.SessionItem, 0, len(sessions))
	for _, session := range sessions {
		title := strings.TrimSpace(session.Title)
		if title == "" {
			title = session.ID
		}
		items = append(items, dialogs.SessionItem{
			ID:           session.ID,
			Title:        title,
			MessageCount: session.MessageCount,
			Current:      session.ID == m.currentSessionID,
		})
	}
	return items
}

func (m *Model) loadSessionMessages(sessionID string) error {
	if m.app == nil || m.app.Messages == nil {
		return nil
	}
	messages, err := m.app.Messages.ListBySession(context.Background(), sessionID)
	if err != nil {
		return err
	}
	m.messages = nil
	m.nextID = 0
	for _, msg := range messages {
		m.addMessage(messageRole(msg.Role), msg.ToolName, msg.Text)
	}
	m.follow()
	return nil
}

func messageRole(role string) viewmodel.Role {
	switch role {
	case messagestore.RoleAssistant:
		return viewmodel.RoleAssistant
	case messagestore.RoleTool:
		return viewmodel.RoleTool
	case messagestore.RoleError:
		return viewmodel.RoleError
	default:
		return viewmodel.RoleUser
	}
}

func firstLine(text string) string {
	text = strings.TrimSpace(text)
	if text == "" {
		return ""
	}
	line, _, _ := strings.Cut(text, "\n")
	return line
}

func promptTitle(title string, name string) string {
	if strings.TrimSpace(title) != "" {
		return title
	}
	return "MCP " + name
}

func firstNonEmpty(values ...string) string {
	for _, value := range values {
		if trimmed := strings.TrimSpace(value); trimmed != "" {
			return trimmed
		}
	}
	return ""
}
