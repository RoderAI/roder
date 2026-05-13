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

	composerHeight := max(3, vm.InputHeight+2)
	reasoningHeight := ReasoningSummaryHeight(vm.ReasoningSummary, height)
	errorHeight := 0
	if vm.ShowErrorLog {
		errorHeight = errorConsoleHeight(height)
	}
	attachmentHeight := 0
	if len(vm.Attachments) > 0 {
		attachmentHeight = 1
	}
	bodyHeight := max(1, height-composerHeight-reasoningHeight-attachmentHeight-errorHeight-2)

	parts := []string{
		Header(width, vm.Provider, vm.Model, vm.Reasoning, vm.SessionTitle, vm.Running),
		TranscriptWithCache(width, bodyHeight, vm.Messages, vm.ScrollOffset, vm.HoveredID, zones, transcriptCache),
	}
	if reasoningHeight > 0 {
		parts = append(parts, ReasoningSummary(width, reasoningHeight, vm.ReasoningSummary))
	}
	if attachmentHeight > 0 {
		parts = append(parts, AttachmentBar(width, vm.Attachments))
	}
	parts = append(parts, Composer(width, vm.Input))
	if vm.ShowErrorLog {
		parts = append(parts, ErrorConsole(width, errorHeight, vm.ErrorLog))
	}
	parts = append(parts, Footer(width, vm.ScrollOffset, vm.Status, vm.ShowErrorLog, len(vm.ErrorLog)))

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
	return view
}

func errorConsoleHeight(totalHeight int) int {
	return min(14, max(5, totalHeight/3))
}

func max(a, b int) int {
	if a > b {
		return a
	}
	return b
}
