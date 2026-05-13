package tui

import (
	"testing"
	"time"

	tea "charm.land/bubbletea/v2"
	"github.com/pandelisz/gode/internal/godex"
	"github.com/pandelisz/gode/internal/tui/dialogs"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

func TestSelectionMouseModeUsesCaptureByDefault(t *testing.T) {
	model := New(nil)
	model.selectionCaptureEnabled = true

	if got := model.selectionMouseMode(time.Now()); got != tea.MouseModeAllMotion {
		t.Fatalf("mouse mode = %v", got)
	}
}

func TestWheelBoundaryTemporarilyDisablesMouseCapture(t *testing.T) {
	model := mouseCaptureTestModel()
	model.scrollOffset = model.maxScrollOffset()
	_ = model.View()
	wheel := transcriptWheel(t, model, tea.MouseWheelUp)

	updated, cmd := model.Update(wheel)
	got := updated.(Model)

	if cmd == nil {
		t.Fatal("expected restore command")
	}
	if mode := got.selectionMouseMode(time.Now()); mode != tea.MouseModeNone {
		t.Fatalf("mouse mode = %v, want none", mode)
	}
}

func TestMouseCaptureRestoreOnlyLatestSequence(t *testing.T) {
	model := New(nil)
	model.selectionCaptureEnabled = true
	model.disableMouseCaptureTemporarily(time.Now())
	oldSeq := model.captureRestoreSeq
	model.disableMouseCaptureTemporarily(time.Now())

	updated, _ := model.Update(mouseCaptureRestoreMsg{seq: oldSeq})
	got := updated.(Model)
	if got.selectionMouseMode(time.Now()) != tea.MouseModeNone {
		t.Fatal("stale restore re-enabled mouse capture")
	}

	updated, _ = got.Update(mouseCaptureRestoreMsg{seq: got.captureRestoreSeq})
	got = updated.(Model)
	if got.selectionMouseMode(time.Now()) != tea.MouseModeAllMotion {
		t.Fatal("latest restore did not re-enable mouse capture")
	}
}

func TestSelectionMouseModeExplicitOverrideDisablesCapture(t *testing.T) {
	model := New(nil)
	model.selectionCaptureEnabled = false

	if got := model.selectionMouseMode(time.Now()); got != tea.MouseModeNone {
		t.Fatalf("mouse mode = %v", got)
	}
}

func TestSelectionMouseModeKeepsDialogMouseCapture(t *testing.T) {
	model := New(nil)
	model.selectionCaptureEnabled = false
	model.settings = dialogs.NewSettings(godex.Config{Provider: "mock", Model: "mock"})
	model.settings.Open = true

	if got := model.selectionMouseMode(time.Now()); got != tea.MouseModeAllMotion {
		t.Fatalf("dialog mouse mode = %v", got)
	}
}

func mouseCaptureTestModel() Model {
	model := New(nil)
	model.width = 80
	model.height = 12
	for i := 0; i < 20; i++ {
		model.messages = append(model.messages, viewmodel.Message{
			ID:   "m",
			Role: viewmodel.RoleAssistant,
			Body: "scroll line",
		})
	}
	model.markTranscriptLinesDirty()
	return model
}

func transcriptWheel(t *testing.T, model Model, button tea.MouseButton) tea.MouseWheelMsg {
	t.Helper()
	z := model.zones.Get(viewmodel.TranscriptZoneID)
	if z == nil || z.IsZero() {
		t.Fatalf("transcript zone missing")
	}
	return tea.MouseWheelMsg{X: z.StartX + 1, Y: z.StartY + 1, Button: button}
}
