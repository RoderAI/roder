package tui

import (
	"time"

	tea "charm.land/bubbletea/v2"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

const mouseCaptureDisableWindow = 900 * time.Millisecond

type mouseCaptureRestoreMsg struct {
	seq int
}

func (m Model) selectionMouseMode(now time.Time) tea.MouseMode {
	if m.dialogNeedsMouseCapture() {
		return tea.MouseModeAllMotion
	}
	if !m.selectionCaptureEnabled {
		return tea.MouseModeNone
	}
	if !m.captureDisabledUntil.IsZero() && now.Before(m.captureDisabledUntil) {
		return tea.MouseModeNone
	}
	return tea.MouseModeAllMotion
}

func (m *Model) disableMouseCaptureTemporarily(now time.Time) tea.Cmd {
	m.captureRestoreSeq++
	seq := m.captureRestoreSeq
	m.captureDisabledUntil = now.Add(mouseCaptureDisableWindow)
	return tea.Tick(mouseCaptureDisableWindow, func(time.Time) tea.Msg {
		return mouseCaptureRestoreMsg{seq: seq}
	})
}

func (m *Model) handleWheel(msg tea.MouseWheelMsg) tea.Cmd {
	if transcript := m.zones.Get(viewmodel.TranscriptZoneID); transcript != nil && !transcript.InBounds(msg) {
		return nil
	}
	mouse := msg.Mouse()
	now := time.Now()
	switch mouse.Button {
	case tea.MouseWheelUp:
		if m.scrollOffset >= m.maxScrollOffset() {
			return m.disableMouseCaptureTemporarily(now)
		}
		m.scrollBy(wheelScrollLines)
	case tea.MouseWheelDown:
		if m.scrollOffset <= 0 {
			return m.disableMouseCaptureTemporarily(now)
		}
		m.scrollBy(-wheelScrollLines)
	}
	return nil
}

func (m *Model) restoreMouseCapture(msg mouseCaptureRestoreMsg) {
	if msg.seq != m.captureRestoreSeq {
		return
	}
	m.captureDisabledUntil = time.Time{}
}

func (m Model) dialogNeedsMouseCapture() bool {
	return m.settings.Open ||
		m.permissions.Open ||
		m.completions.Open ||
		m.commands.Open ||
		m.sessions.Open ||
		m.inlineSlashMenuOpen()
}
