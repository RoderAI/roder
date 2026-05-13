package tui

import (
	"testing"

	tea "charm.land/bubbletea/v2"
	"github.com/pandelisz/gode/internal/tui/selection"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

func TestSelectionIntegrationTranscriptPromptAppend(t *testing.T) {
	model := New(nil)
	model.input.SetValue("existing")
	model.transcriptSelection = selection.Range{
		Anchor: selection.Point{Line: 0, Column: 0},
		Focus:  selection.Point{Line: 0, Column: 12},
		Active: true,
	}
	model.transcriptLineRefs = []selection.TranscriptLineRef{{DisplayLine: 0, MessageIndex: 0, CopyText: "selected text"}}

	updated, _ := model.Update(tea.KeyPressMsg{Code: 'p', Text: "p"})
	got := updated.(Model)

	if got.input.Value() != "existing\n\nselected text" {
		t.Fatalf("input = %q", got.input.Value())
	}
}

func TestSelectionIntegrationComposerCopyThenType(t *testing.T) {
	model := New(nil)
	model.input.SetValue("copy then type")
	model.composerSelection = selection.OffsetRange{Anchor: 0, Focus: 4, Active: true}
	var copied string
	model.clipboardWrite = func(text string) error {
		copied = text
		return nil
	}

	updated, _ := model.Update(tea.KeyPressMsg{Code: 'c', Text: "c"})
	got := updated.(Model)
	updated, _ = got.Update(tea.KeyPressMsg{Code: '!', Text: "!"})
	got = updated.(Model)

	if copied != "copy" {
		t.Fatalf("copied = %q", copied)
	}
	if got.composerSelection.Active || got.input.Value() != "copy then type!" {
		t.Fatalf("selection/input = %#v %q", got.composerSelection, got.input.Value())
	}
}

func TestSelectionIntegrationTranscriptSelectionClearsOnlyWhenViewportLineLeaves(t *testing.T) {
	model := New(nil)
	model.width = 60
	model.height = 10
	model.messages = []viewmodel.Message{{ID: "m1", Role: viewmodel.RoleAssistant, Body: "selected"}}
	model.refreshTranscriptLineRefs()
	model.transcriptSelection = selection.Range{
		Anchor: selection.Point{Line: 0, Column: 0},
		Focus:  selection.Point{Line: 0, Column: 8},
		Active: true,
	}

	model.appendAssistantDelta(" still visible")
	if !model.transcriptSelection.Active {
		t.Fatal("selection should remain while selected row stays visible")
	}

	for i := 0; i < 20; i++ {
		model.addMessage(viewmodel.RoleAssistant, "", "new row")
	}
	if model.transcriptSelection.Active {
		t.Fatalf("selection should clear after selected row leaves viewport: %#v", model.transcriptSelection)
	}
}

func TestSelectionIntegrationSettingsDialogDoesNotMutateSelection(t *testing.T) {
	model := New(nil)
	model.width = 80
	model.height = 20
	model.transcriptSelection = selection.Range{
		Anchor: selection.Point{Line: 0, Column: 0},
		Focus:  selection.Point{Line: 0, Column: 5},
		Active: true,
	}
	model.openSettings()
	_ = model.View()

	updated, _ := model.Update(clickZone(t, model, viewmodel.SettingsMenuItemZoneID("config")))
	got := updated.(Model)

	if !got.transcriptSelection.Active {
		t.Fatalf("settings click should not clear transcript selection: %#v", got.transcriptSelection)
	}
	if !got.settings.Open {
		t.Fatal("settings dialog should consume click")
	}
}
