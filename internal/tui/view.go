package tui

import (
	"time"

	tea "charm.land/bubbletea/v2"
	"github.com/pandelisz/gode/internal/godex"
	"github.com/pandelisz/gode/internal/tui/components"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

func (m Model) View() tea.View {
	vm := m.viewModel()
	view := tea.NewView(m.zones.Scan(components.RenderWithCache(vm, m.zones, &m.transcript)))
	view.AltScreen = true
	view.MouseMode = m.selectionMouseMode(time.Now())
	view.WindowTitle = "gode"
	return view
}

func (m Model) viewModel() viewmodel.Model {
	settings := m.settings.ViewModel()
	commands := m.commandsViewModel()
	sessions := m.sessionsViewModel()
	completions := m.completionsViewModel()
	permissions := m.permissionsViewModel()
	vm := viewmodel.Model{
		Width:                     m.width,
		Height:                    m.height,
		Messages:                  m.messages,
		ReasoningSummary:          m.reasoningSummary,
		QueuedPrompts:             m.queuedPromptDisplays(),
		Attachments:               m.attachmentViewModels(),
		Input:                     m.input.View(),
		ComposerValue:             m.input.Value(),
		InputHeight:               m.input.Height(),
		SlashMenu:                 m.slashMenuViewModel(),
		ScrollOffset:              m.scrollOffset,
		FollowTail:                m.followTail,
		TranscriptSelection:       m.transcriptSelection,
		TranscriptSelectionActive: m.transcriptSelection.Active,
		TranscriptSelectionHint:   m.transcriptSelectionHint(),
		CopyNotice:                m.copyNotice(),
		ComposerSelection:         m.composerSelection,
		ComposerSelectionActive:   m.composerSelection.Active,
		ComposerSelectionHint:     m.composerSelectionHint(),
		Running:                   m.running,
		HoveredID:                 m.hoveredID,
		Status:                    m.footerStatus(),
		ContextLeft:               m.contextLeft,
		SessionTitle:              m.currentSession,
		Dialogs: viewmodel.DialogStack{
			Settings:    settings,
			Completions: completions,
			Commands:    commands,
			Sessions:    sessions,
			Permissions: permissions,
		},
		Settings:     settings,
		QuitDialog:   m.quitConfirmViewModel(),
		ErrorLog:     m.errorLog,
		ShowErrorLog: m.showErrorLog,
	}
	if m.app != nil {
		vm.Provider = godex.DisplayProvider(m.app.Config)
		vm.Model = m.app.Config.Model
		vm.Reasoning = m.app.Config.Reasoning
		vm.AutoApprove = m.app.Config.AutoApprove
		vm.TimelineStyle = m.app.Config.TimelineStyle
		vm.MarkdownRendering = m.app.Config.MarkdownRendering
	}
	return vm
}
