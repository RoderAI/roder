package tui

import (
	"strings"

	tea "charm.land/bubbletea/v2"
	"github.com/charmbracelet/x/ansi"
	"github.com/pandelisz/gode/internal/tui/components"
	"github.com/pandelisz/gode/internal/tui/selection"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

func (m *Model) startTranscriptSelection(msg tea.MouseClickMsg) bool {
	if msg.Button != tea.MouseLeft {
		return false
	}
	point, ok := m.transcriptPointForMouse(msg)
	if !ok {
		return false
	}
	m.transcriptSelection = selection.Range{Anchor: point, Focus: point, Active: true}
	m.transcriptMouseDown = true
	m.status = m.transcriptSelectionHint()
	return true
}

func (m *Model) updateTranscriptSelectionDrag(msg tea.MouseMotionMsg) bool {
	if !m.transcriptMouseDown || msg.Button != tea.MouseLeft {
		return false
	}
	point, ok := m.transcriptPointForMouse(msg)
	if !ok {
		return true
	}
	m.transcriptSelection.Focus = point
	m.transcriptSelection.Active = true
	m.status = m.transcriptSelectionHint()
	return true
}

func (m *Model) finishTranscriptSelection(msg tea.MouseReleaseMsg) bool {
	if !m.transcriptMouseDown {
		return false
	}
	m.transcriptMouseDown = false
	if point, ok := m.transcriptPointForMouse(msg); ok {
		m.transcriptSelection.Focus = point
	}
	if !m.transcriptSelection.CanCopy(m.transcriptSelectionTextLines()) {
		m.transcriptSelection = selection.Range{}
		m.status = "ready"
		return true
	}
	m.transcriptSelection.Active = true
	m.status = m.transcriptSelectionHint()
	return true
}

func (m *Model) transcriptPointForMouse(msg tea.MouseMsg) (selection.Point, bool) {
	m.refreshTranscriptLineRefs()
	transcript := m.zones.Get(viewmodel.TranscriptZoneID)
	if transcript == nil || !transcript.InBounds(msg) {
		return selection.Point{}, false
	}
	x, y := transcript.Pos(msg)
	line := y - 1
	if line < 0 || line >= len(m.transcriptLineRefs) {
		return selection.Point{}, false
	}
	text := m.transcriptLineRefs[line].Text
	column := clamp(x-1, 0, len([]rune(ansi.Strip(text))))
	return selection.Point{Line: line, Column: column}, true
}

func (m *Model) refreshTranscriptLineRefs() {
	height := max(4, m.visibleTranscriptLines()+2)
	timelineStyle := ""
	if m.app != nil {
		timelineStyle = m.app.Config.TimelineStyle
	}
	m.transcriptLineRefs = components.TranscriptLineRefs(m.width, height, m.messages, m.scrollOffset, &m.transcript, timelineStyle)
}

func (m *Model) transcriptSelectionTextLines() []string {
	if len(m.transcriptLineRefs) == 0 {
		m.refreshTranscriptLineRefs()
	}
	lines := make([]string, len(m.transcriptLineRefs))
	for i, line := range m.transcriptLineRefs {
		if line.CopyText != "" {
			lines[i] = line.CopyText
		} else {
			lines[i] = ansi.Strip(line.Text)
		}
	}
	return lines
}

func (m Model) transcriptSelectionHint() string {
	if !m.transcriptSelection.Active {
		return ""
	}
	return "c copy | p prompt | esc clear"
}

func selectedTranscriptRefs(refs []selection.TranscriptLineRef, selected selection.Range) []selection.TranscriptLineRef {
	start, end, ok := selected.Normalize()
	if !ok {
		return nil
	}
	out := make([]selection.TranscriptLineRef, 0, end.Line-start.Line+1)
	for _, ref := range refs {
		if ref.DisplayLine < start.Line || ref.DisplayLine > end.Line {
			continue
		}
		out = append(out, ref)
	}
	return out
}

func selectedTranscriptCopyText(refs []selection.TranscriptLineRef, selected selection.Range) string {
	return strings.TrimSpace(selection.SanitizeTranscriptCopy(selectedTranscriptRefs(refs, selected)))
}

func (m *Model) reconcileTranscriptSelection() {
	if !m.transcriptSelection.Active {
		return
	}
	selectedBefore := selectedTranscriptRefs(m.transcriptLineRefs, m.transcriptSelection)
	if len(selectedBefore) == 0 {
		return
	}
	m.refreshTranscriptLineRefs()
	selectedAfter := selectedTranscriptRefs(m.transcriptLineRefs, m.transcriptSelection)
	if len(selectedAfter) != len(selectedBefore) {
		m.transcriptSelection = selection.Range{}
		return
	}
	for i := range selectedBefore {
		if selectedBefore[i].MessageIndex != selectedAfter[i].MessageIndex ||
			selectedBefore[i].LogicalLine != selectedAfter[i].LogicalLine ||
			selectedBefore[i].Decorative != selectedAfter[i].Decorative {
			m.transcriptSelection = selection.Range{}
			return
		}
	}
}
