package components

import (
	"charm.land/lipgloss/v2"
	zone "github.com/lrstanley/bubblezone/v2"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

func Render(vm viewmodel.Model, zones *zone.Manager) string {
	return RenderWithCache(vm, zones, nil)
}

func RenderWithCache(vm viewmodel.Model, zones *zone.Manager, transcriptCache *TranscriptCache) string {
	width := max(40, vm.Width)
	height := max(12, vm.Height)

	reasoningHeight := ReasoningSummaryHeight(vm.ReasoningSummary, height)
	errorHeight := 0
	if vm.ShowErrorLog {
		errorHeight = errorConsoleHeight(height)
	}
	attachmentHeight := 0
	if len(vm.Attachments) > 0 {
		attachmentHeight = 1
	}

	header := Header(width, vm.Provider, vm.Model, vm.Reasoning, vm.SessionTitle, vm.Running)
	reasoning := ""
	if reasoningHeight > 0 {
		reasoning = ReasoningSummary(width, reasoningHeight, vm.ReasoningSummary)
	}
	attachment := ""
	if attachmentHeight > 0 {
		attachment = AttachmentBar(width, vm.Attachments)
	}
	composer := ComposerWithSelection(width, vm.Input, ComposerOptions{
		Value:       vm.ComposerValue,
		Selection:   vm.ComposerSelection,
		AutoApprove: vm.AutoApprove,
	}, zones)
	slashMenu := ""
	if vm.SlashMenu != nil {
		slashMenu = InlineListDialog(width, *vm.SlashMenu, zones)
	}
	errorLog := ""
	if vm.ShowErrorLog {
		errorLog = ErrorConsole(width, errorHeight, vm.ErrorLog)
	}
	footer := Footer(width, vm.ScrollOffset, vm.Status, vm.ShowErrorLog, len(vm.ErrorLog), vm.ContextLeft)
	reservedHeight := renderedHeight(header) + renderedHeight(reasoning) + renderedHeight(attachment) + renderedHeight(composer) + renderedHeight(slashMenu) + renderedHeight(errorLog) + renderedHeight(footer)
	bodyHeight := max(1, height-reservedHeight)

	parts := []string{
		header,
		TranscriptDetailedWithCache(width, bodyHeight, vm.Messages, vm.ScrollOffset, vm.HoveredID, zones, transcriptCache, TranscriptOptions{
			Selection:         vm.TranscriptSelection,
			TimelineStyle:     vm.TimelineStyle,
			MarkdownRendering: vm.MarkdownRendering,
		}).View,
	}
	for _, part := range []string{reasoning, attachment, composer, slashMenu, errorLog, footer} {
		if part != "" {
			parts = append(parts, part)
		}
	}

	view := lipgloss.JoinVertical(
		lipgloss.Left,
		parts...,
	)
	if settings := vm.ActiveSettingsDialog(); settings != nil {
		return OverlaySettingsDialog(view, width, height, *settings, zones)
	}
	if permissions := vm.ActivePermissionDialog(); permissions != nil {
		return OverlayPermissionDialog(view, width, height, *permissions, zones)
	}
	if list := vm.ActiveListDialog(); list != nil {
		return OverlayListDialog(view, width, height, *list, zones)
	}
	if vm.QuitDialog != nil {
		return OverlayConfirmDialog(view, width, height, *vm.QuitDialog)
	}
	return view
}

func errorConsoleHeight(totalHeight int) int {
	return min(14, max(5, totalHeight/3))
}

func renderedHeight(view string) int {
	if view == "" {
		return 0
	}
	return lipgloss.Height(view)
}

func max(a, b int) int {
	if a > b {
		return a
	}
	return b
}
