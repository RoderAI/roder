package tui

import (
	"strings"
	"testing"
	"time"

	tea "charm.land/bubbletea/v2"
	"github.com/pandelisz/gode/internal/tui/selection"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

func TestTranscriptSelectionMouseDownMapsViewportCoordinateToLineRef(t *testing.T) {
	model := transcriptSelectionTestModel("hello transcript")
	click := transcriptMouseClick(t, model, 3, 1)

	updated, _ := model.Update(click)
	got := updated.(Model)

	if !got.transcriptSelection.Active || got.transcriptSelection.Anchor.Line != 0 {
		t.Fatalf("selection = %#v", got.transcriptSelection)
	}
	if len(got.transcriptLineRefs) == 0 || !strings.Contains(got.transcriptLineRefs[0].CopyText, "hello transcript") {
		t.Fatalf("line refs = %#v", got.transcriptLineRefs)
	}
}

func TestTranscriptSelectionMouseDragUpdatesAcrossWrappedLines(t *testing.T) {
	model := transcriptSelectionTestModel("first wrapped line second wrapped line third wrapped line")
	click := transcriptMouseClick(t, model, 2, 1)
	updated, _ := model.Update(click)
	got := updated.(Model)

	drag := tea.MouseMotionMsg{X: click.X + 6, Y: click.Y + 1, Button: tea.MouseLeft}
	updated, _ = got.Update(drag)
	got = updated.(Model)

	if !got.transcriptSelection.Active || got.transcriptSelection.Focus.Line <= got.transcriptSelection.Anchor.Line {
		t.Fatalf("drag selection = %#v", got.transcriptSelection)
	}
}

func TestTranscriptSelectionMouseReleaseFinalizesOrClears(t *testing.T) {
	model := transcriptSelectionTestModel("hello transcript")
	click := transcriptMouseClick(t, model, 2, 1)
	updated, _ := model.Update(click)
	got := updated.(Model)

	release := tea.MouseReleaseMsg{X: click.X + 5, Y: click.Y, Button: tea.MouseLeft}
	updated, _ = got.Update(release)
	got = updated.(Model)
	if !got.transcriptSelection.Active || got.transcriptMouseDown {
		t.Fatalf("expected finalized active selection: %#v mouseDown=%v", got.transcriptSelection, got.transcriptMouseDown)
	}

	updated, _ = got.Update(click)
	got = updated.(Model)
	shortRelease := tea.MouseReleaseMsg{X: click.X + 1, Y: click.Y, Button: tea.MouseLeft}
	updated, _ = got.Update(shortRelease)
	got = updated.(Model)
	if got.transcriptSelection.Active {
		t.Fatalf("short selection should clear: %#v", got.transcriptSelection)
	}
}

func TestTranscriptSelectionViewModelFields(t *testing.T) {
	model := transcriptSelectionTestModel("hello transcript")
	model.transcriptSelection = selection.Range{
		Anchor: selection.Point{Line: 0, Column: 0},
		Focus:  selection.Point{Line: 0, Column: 5},
		Active: true,
	}
	vm := model.viewModel()

	if !vm.TranscriptSelectionActive || vm.TranscriptSelectionHint == "" || vm.CopyNotice != "" {
		t.Fatalf("selection viewmodel fields not populated: %#v", vm)
	}
}

func transcriptSelectionTestModel(body string) Model {
	model := New(nil)
	model.width = 48
	model.height = 14
	model.messages = []viewmodel.Message{{
		ID:   "m1",
		Role: viewmodel.RoleAssistant,
		Body: body,
	}}
	_ = model.View()
	return model
}

func transcriptMouseClick(t *testing.T, model Model, x int, y int) tea.MouseClickMsg {
	t.Helper()
	z := model.zones.Get(viewmodel.TranscriptZoneID)
	deadline := time.Now().Add(100 * time.Millisecond)
	for (z == nil || z.IsZero()) && time.Now().Before(deadline) {
		time.Sleep(time.Millisecond)
		z = model.zones.Get(viewmodel.TranscriptZoneID)
	}
	if z == nil || z.IsZero() {
		t.Fatalf("transcript zone missing")
	}
	return tea.MouseClickMsg{X: z.StartX + x, Y: z.StartY + y, Button: tea.MouseLeft}
}
