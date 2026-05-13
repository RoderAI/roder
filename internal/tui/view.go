package tui

import (
	tea "charm.land/bubbletea/v2"
	"github.com/pandelisz/gode/internal/godex"
	"github.com/pandelisz/gode/internal/tui/components"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

func (m Model) View() tea.View {
	settings := m.settings.ViewModel()
	commands := m.commandsViewModel()
	sessions := m.sessionsViewModel()
	completions := m.completionsViewModel()
	permissions := m.permissionsViewModel()
	vm := viewmodel.Model{
		Width:            m.width,
		Height:           m.height,
		Messages:         m.messages,
		ReasoningSummary: m.reasoningSummary,
		Attachments:      m.attachmentViewModels(),
		Input:            m.input.View(),
		InputHeight:      m.input.Height(),
		ScrollOffset:     m.scrollOffset,
		FollowTail:       m.followTail,
		Running:          m.running,
		HoveredID:        m.hoveredID,
		Status:           m.status,
		SessionTitle:     m.currentSession,
		Dialogs: viewmodel.DialogStack{
			Settings:    settings,
			Completions: completions,
			Commands:    commands,
			Sessions:    sessions,
			Permissions: permissions,
		},
		Settings:     settings,
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
