package components

import (
	"strings"

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
	bodyHeight := max(1, height-composerHeight-reasoningHeight-errorHeight-3)

	parts := []string{
		Header(width, vm.Provider, vm.Model, vm.Reasoning, vm.Running),
		TranscriptWithCache(width, bodyHeight, vm.Messages, vm.ScrollOffset, vm.HoveredID, zones, transcriptCache),
	}
	if reasoningHeight > 0 {
		parts = append(parts, ReasoningSummary(width, reasoningHeight, vm.ReasoningSummary))
	}
	parts = append(parts, Composer(width, vm.Input))
	if vm.ShowErrorLog {
		parts = append(parts, ErrorConsole(width, errorHeight, vm.ErrorLog))
	}
	parts = append(parts, Footer(width, vm.ScrollOffset, vm.Status, vm.ShowErrorLog, len(vm.ErrorLog)))
	parts = append(parts, BottomGutter(width))

	view := lipgloss.JoinVertical(
		lipgloss.Left,
		parts...,
	)
	if settings := vm.ActiveSettingsDialog(); settings != nil {
		return OverlaySettingsDialog(view, width, height, *settings, zones)
	}
	return view
}

func errorConsoleHeight(totalHeight int) int {
	return min(14, max(5, totalHeight/3))
}

func BottomGutter(width int) string {
	return strings.Repeat(" ", max(1, width-1))
}

func max(a, b int) int {
	if a > b {
		return a
	}
	return b
}
