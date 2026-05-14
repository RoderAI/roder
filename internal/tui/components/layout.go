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

	inlineMenuOpen := vm.SlashMenu != nil || vm.CompletionMenu != nil
	maxInlineRows := maxInlineListRows
	showFooter := true
	if inlineMenuOpen {
		maxInlineRows = maxInlineListRowsWithoutFooter
		showFooter = false
	}
	reasoningHeight := ReasoningSummaryHeight(vm.ReasoningSummary, height)
	errorHeight := 0
	if vm.ShowErrorLog {
		errorHeight = errorConsoleHeight(height)
	}
	attachmentHeight := 0
	if len(vm.Attachments) > 0 {
		attachmentHeight = 1
	}
	queuedHeight := QueuedPromptsHeight(vm.QueuedPrompts)
	header := Header(width, vm.Provider, vm.Model, vm.Reasoning, vm.SessionTitle, vm.Running)
	reasoning := ""
	if reasoningHeight > 0 {
		reasoning = ReasoningSummary(width, reasoningHeight, vm.ReasoningSummary)
	}
	attachment := ""
	if attachmentHeight > 0 {
		attachment = AttachmentBar(width, vm.Attachments)
	}
	queuedPrompts := ""
	if queuedHeight > 0 {
		queuedPrompts = QueuedPrompts(width, vm.QueuedPrompts)
	}
	composer := ComposerWithSelection(width, vm.Input, ComposerOptions{
		Value:       vm.ComposerValue,
		Selection:   vm.ComposerSelection,
		AutoApprove: vm.AutoApprove,
	}, zones)
	slashMenu := ""
	if vm.SlashMenu != nil {
		slashMenu = InlineListDialogWithRows(width, *vm.SlashMenu, maxInlineRows, zones)
	}
	completionMenu := ""
	if vm.CompletionMenu != nil {
		completionMenu = InlineListDialogWithRows(width, *vm.CompletionMenu, maxInlineRows, zones)
	}
	errorLog := ""
	if vm.ShowErrorLog {
		errorLog = ErrorConsole(width, errorHeight, vm.ErrorLog)
	}
	footer := ""
	if showFooter {
		footer = Footer(width, vm.ScrollOffset, vm.Status, vm.ShowErrorLog, len(vm.ErrorLog), vm.ContextLeft)
	}
	reservedHeight := renderedHeight(header) + renderedHeight(reasoning) + renderedHeight(attachment) + renderedHeight(queuedPrompts) + renderedHeight(composer) + renderedHeight(slashMenu) + renderedHeight(completionMenu) + renderedHeight(errorLog) + renderedHeight(footer)
	bodyHeight := max(0, height-reservedHeight)

	transcript := ""
	if bodyHeight > 0 {
		transcript = TranscriptDetailedWithCache(width, bodyHeight, vm.Messages, vm.ScrollOffset, vm.HoveredID, zones, transcriptCache, TranscriptOptions{
			Selection:         vm.TranscriptSelection,
			TimelineStyle:     vm.TimelineStyle,
			MarkdownRendering: vm.MarkdownRendering,
		}).View
	}
	parts := []string{header}
	if transcript != "" {
		parts = append(parts, transcript)
	}
	for _, part := range []string{reasoning, attachment, queuedPrompts, composer, slashMenu, completionMenu, errorLog, footer} {
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
	if remote := vm.ActiveRemoteDialog(); remote != nil {
		return OverlayRemoteDialog(view, width, height, *remote)
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
