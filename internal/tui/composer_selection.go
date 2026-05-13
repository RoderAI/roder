package tui

import (
	tea "charm.land/bubbletea/v2"
	"github.com/pandelisz/gode/internal/tui/components"
	"github.com/pandelisz/gode/internal/tui/selection"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

func (m *Model) startComposerSelection(msg tea.MouseClickMsg) bool {
	if msg.Button != tea.MouseLeft {
		return false
	}
	offset, ok := m.composerOffsetForMouse(msg)
	if !ok {
		return false
	}
	m.composerSelection = selection.OffsetRange{Anchor: offset, Focus: offset, Active: true}
	m.composerMouseDown = true
	m.transcriptSelection = selection.Range{}
	m.status = m.composerSelectionHint()
	return true
}

func (m *Model) updateComposerSelectionDrag(msg tea.MouseMotionMsg) bool {
	if !m.composerMouseDown || msg.Button != tea.MouseLeft {
		return false
	}
	offset, ok := m.composerOffsetForMouse(msg)
	if !ok {
		return true
	}
	m.composerSelection.Focus = offset
	m.composerSelection.Active = true
	m.status = m.composerSelectionHint()
	return true
}

func (m *Model) finishComposerSelection(msg tea.MouseReleaseMsg) bool {
	if !m.composerMouseDown {
		return false
	}
	m.composerMouseDown = false
	if offset, ok := m.composerOffsetForMouse(msg); ok {
		m.composerSelection.Focus = offset
	}
	if !m.composerSelection.CanCopy(m.input.Value()) {
		m.composerSelection = selection.OffsetRange{}
		m.status = "ready"
		return true
	}
	m.composerSelection.Active = true
	m.status = m.composerSelectionHint()
	return true
}

func (m *Model) composerOffsetForMouse(msg tea.MouseMsg) (int, bool) {
	composer := m.zones.Get(viewmodel.ComposerZoneID)
	if composer == nil || !composer.InBounds(msg) {
		return 0, false
	}
	x, y := composer.Pos(msg)
	row := y - 1
	col := x - 2
	if row < 0 {
		return 0, false
	}
	return components.ComposerOffsetAt(m.input.Value(), m.width, row, col), true
}

func (m Model) composerSelectionHint() string {
	if !m.composerSelection.Active {
		return ""
	}
	return "c copy | esc clear"
}
