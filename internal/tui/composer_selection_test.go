package tui

import (
	"testing"
	"time"

	tea "charm.land/bubbletea/v2"
	"github.com/pandelisz/gode/internal/tui/selection"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

func TestComposerSelectionMouseDownMapsVisualCellToOffset(t *testing.T) {
	model := composerSelectionTestModel("hello composer")
	click := composerMouseClick(t, model, 4, 1)

	updated, _ := model.Update(click)
	got := updated.(Model)

	if !got.composerSelection.Active || got.composerSelection.Anchor != 2 {
		t.Fatalf("composer selection = %#v", got.composerSelection)
	}
}

func TestComposerSelectionDragAcrossWrappedRowsSelectsText(t *testing.T) {
	model := composerSelectionTestModel("abcdefghij klmnopqrst")
	model.width = 18
	_ = model.View()
	click := composerMouseClick(t, model, 2, 1)
	updated, _ := model.Update(click)
	got := updated.(Model)

	drag := tea.MouseMotionMsg{X: click.X + 3, Y: click.Y + 1, Button: tea.MouseLeft}
	updated, _ = got.Update(drag)
	got = updated.(Model)

	selected := got.composerSelection.SelectedText(got.input.Value())
	if selected == "" || selected == got.input.Value() {
		t.Fatalf("selected text = %q from range %#v", selected, got.composerSelection)
	}
}

func TestComposerSelectionMouseReleaseClearsCollapsedOrShortSelection(t *testing.T) {
	model := composerSelectionTestModel("hello")
	click := composerMouseClick(t, model, 4, 1)
	updated, _ := model.Update(click)
	got := updated.(Model)

	release := tea.MouseReleaseMsg{X: click.X + 1, Y: click.Y, Button: tea.MouseLeft}
	updated, _ = got.Update(release)
	got = updated.(Model)

	if got.composerSelection.Active {
		t.Fatalf("short composer selection should clear: %#v", got.composerSelection)
	}
}

func TestComposerSelectionViewModelFieldsAndClearsTranscriptSelection(t *testing.T) {
	model := composerSelectionTestModel("hello")
	model.transcriptSelection = selection.Range{
		Anchor: selection.Point{Line: 0, Column: 0},
		Focus:  selection.Point{Line: 0, Column: 5},
		Active: true,
	}
	click := composerMouseClick(t, model, 2, 1)
	updated, _ := model.Update(click)
	got := updated.(Model)
	vm := got.viewModel()

	if got.transcriptSelection.Active {
		t.Fatalf("transcript selection was not cleared: %#v", got.transcriptSelection)
	}
	if !vm.ComposerSelectionActive || vm.ComposerSelectionHint == "" {
		t.Fatalf("composer selection fields missing: %#v", vm)
	}
}

func composerSelectionTestModel(value string) Model {
	model := New(nil)
	model.width = 48
	model.height = 14
	model.input.SetValue(value)
	_ = model.View()
	return model
}

func composerMouseClick(t *testing.T, model Model, x int, y int) tea.MouseClickMsg {
	t.Helper()
	z := model.zones.Get(viewmodel.ComposerZoneID)
	deadline := time.Now().Add(100 * time.Millisecond)
	for (z == nil || z.IsZero()) && time.Now().Before(deadline) {
		time.Sleep(time.Millisecond)
		z = model.zones.Get(viewmodel.ComposerZoneID)
	}
	if z == nil || z.IsZero() {
		t.Fatalf("composer zone missing")
	}
	return tea.MouseClickMsg{X: z.StartX + x, Y: z.StartY + y, Button: tea.MouseLeft}
}
