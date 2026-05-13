package tui

import (
	"strings"
	"time"

	tea "charm.land/bubbletea/v2"
	"github.com/pandelisz/gode/internal/tui/selection"
)

func (m *Model) handleSelectionKey(msg tea.KeyPressMsg) bool {
	if !m.hasActiveSelection() {
		return false
	}
	switch msg.String() {
	case "esc", "escape", "ctrl+[":
		m.clearSelections()
		m.status = "ready"
		return true
	case "c":
		m.copyActiveSelection()
		return true
	case "p":
		if m.transcriptSelection.Active {
			m.appendTranscriptSelectionToPrompt()
		}
		return true
	default:
		return false
	}
}

func (m *Model) hasActiveSelection() bool {
	return m.transcriptSelection.Active || m.composerSelection.Active
}

func (m *Model) clearSelections() {
	m.transcriptSelection = selection.Range{}
	m.composerSelection = selection.OffsetRange{}
	m.transcriptMouseDown = false
	m.composerMouseDown = false
}

func (m *Model) copyActiveSelection() {
	text := ""
	switch {
	case m.transcriptSelection.Active:
		text = selectedTranscriptCopyText(m.transcriptLineRefs, m.transcriptSelection)
	case m.composerSelection.Active:
		text = m.composerSelection.SelectedText(m.input.Value())
	}
	text = strings.TrimSpace(text)
	if text == "" {
		m.status = "selection empty"
		return
	}
	writer := m.clipboardWrite
	if writer == nil {
		writer = selection.SystemClipboardWriter
	}
	if err := writer(text); err != nil {
		m.status = "clipboard failed - " + truncateStatus(err.Error(), 120)
		return
	}
	m.clearSelections()
	m.copyNoticeUntil = time.Now().Add(time.Second)
	m.status = "Copied to clipboard"
}

func (m *Model) appendTranscriptSelectionToPrompt() {
	text := selectedTranscriptCopyText(m.transcriptLineRefs, m.transcriptSelection)
	text = strings.TrimSpace(text)
	if text == "" {
		return
	}
	current := strings.TrimRight(m.input.Value(), "\n")
	if strings.TrimSpace(current) != "" {
		m.input.SetValue(current + "\n\n" + text)
	} else {
		m.input.SetValue(text)
	}
	m.transcriptSelection = selection.Range{}
	m.transcriptMouseDown = false
	m.status = "selection appended to prompt"
}

func (m Model) copyNotice() string {
	if m.copyNoticeUntil.IsZero() || time.Now().After(m.copyNoticeUntil) {
		return ""
	}
	return "Copied to clipboard"
}
