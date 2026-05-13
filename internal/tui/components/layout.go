package components

import (
	"charm.land/lipgloss/v2"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

func Render(vm viewmodel.Model) string {
	width := max(40, vm.Width)
	bodyHeight := max(4, vm.Height-vm.InputHeight-5)

	return lipgloss.JoinVertical(
		lipgloss.Left,
		Header(width, vm.Provider, vm.Model),
		Transcript(width, bodyHeight, vm.Lines),
		Composer(width, vm.Input),
		Footer(width),
	)
}

func max(a, b int) int {
	if a > b {
		return a
	}
	return b
}
