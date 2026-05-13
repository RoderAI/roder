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
	bodyHeight := max(6, height-composerHeight-2)

	view := lipgloss.JoinVertical(
		lipgloss.Left,
		Header(width, vm.Provider, vm.Model, vm.Running),
		TranscriptWithCache(width, bodyHeight, vm.Messages, vm.ScrollOffset, vm.HoveredID, zones, transcriptCache),
		Composer(width, vm.Input),
		Footer(width, vm.ScrollOffset, vm.Status),
	)
	if vm.Settings != nil {
		return OverlaySettingsDialog(view, width, height, *vm.Settings, zones)
	}
	return view
}

func max(a, b int) int {
	if a > b {
		return a
	}
	return b
}
