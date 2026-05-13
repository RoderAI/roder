package tui

import (
	"errors"
	"strings"
	"testing"

	tea "charm.land/bubbletea/v2"
	"github.com/pandelisz/gode/internal/tui/selection"
)

func TestSelectionKeyCopiesTranscriptToClipboard(t *testing.T) {
	model := New(nil)
	model.transcriptSelection = selection.Range{
		Anchor: selection.Point{Line: 0, Column: 0},
		Focus:  selection.Point{Line: 0, Column: 12},
		Active: true,
	}
	model.transcriptLineRefs = []selection.TranscriptLineRef{{DisplayLine: 0, CopyText: "copy transcript"}}
	var copied string
	model.clipboardWrite = func(text string) error {
		copied = text
		return nil
	}

	updated, _ := model.Update(tea.KeyPressMsg{Code: 'c'})
	got := updated.(Model)

	if copied != "copy transcript" {
		t.Fatalf("copied transcript = %q", copied)
	}
	if got.transcriptSelection.Active || got.viewModel().CopyNotice == "" {
		t.Fatalf("selection/copy notice = %#v %q", got.transcriptSelection, got.viewModel().CopyNotice)
	}
}

func TestSelectionKeyCopiesComposerToClipboard(t *testing.T) {
	model := New(nil)
	model.input.SetValue("copy composer")
	model.composerSelection = selection.OffsetRange{Anchor: 0, Focus: 13, Active: true}
	var copied string
	model.clipboardWrite = func(text string) error {
		copied = text
		return nil
	}

	updated, _ := model.Update(tea.KeyPressMsg{Code: 'c'})
	got := updated.(Model)

	if copied != "copy composer" {
		t.Fatalf("copied composer = %q", copied)
	}
	if got.composerSelection.Active {
		t.Fatalf("composer selection should clear: %#v", got.composerSelection)
	}
}

func TestSelectionKeyEscClearsSelections(t *testing.T) {
	model := New(nil)
	model.transcriptSelection = selection.Range{Anchor: selection.Point{Line: 0}, Focus: selection.Point{Line: 0, Column: 4}, Active: true}
	model.composerSelection = selection.OffsetRange{Anchor: 0, Focus: 4, Active: true}

	updated, _ := model.Update(tea.KeyPressMsg{Code: tea.KeyEscape})
	got := updated.(Model)

	if got.transcriptSelection.Active || got.composerSelection.Active {
		t.Fatalf("selections not cleared: %#v %#v", got.transcriptSelection, got.composerSelection)
	}
}

func TestSelectionKeyPromptAppendFromTranscript(t *testing.T) {
	model := New(nil)
	model.input.SetValue("existing")
	model.transcriptSelection = selection.Range{
		Anchor: selection.Point{Line: 0, Column: 0},
		Focus:  selection.Point{Line: 0, Column: 5},
		Active: true,
	}
	model.transcriptLineRefs = []selection.TranscriptLineRef{{DisplayLine: 0, CopyText: "clean"}}

	updated, _ := model.Update(tea.KeyPressMsg{Code: 'p'})
	got := updated.(Model)

	if got.input.Value() != "existing\n\nclean" {
		t.Fatalf("input = %q", got.input.Value())
	}
	if got.transcriptSelection.Active {
		t.Fatalf("transcript selection should clear: %#v", got.transcriptSelection)
	}
}

func TestSelectionKeyPromptAppendIgnoresComposerSelection(t *testing.T) {
	model := New(nil)
	model.input.SetValue("existing")
	model.composerSelection = selection.OffsetRange{Anchor: 0, Focus: 4, Active: true}

	updated, _ := model.Update(tea.KeyPressMsg{Code: 'p'})
	got := updated.(Model)

	if got.input.Value() != "existing" {
		t.Fatalf("input changed: %q", got.input.Value())
	}
}

func TestSelectionKeyNormalTypingContinuesWithoutSelection(t *testing.T) {
	model := New(nil)
	updated, _ := model.Update(tea.KeyPressMsg{Code: 'x', Text: "x"})
	got := updated.(Model)

	if !strings.Contains(got.input.Value(), "x") {
		t.Fatalf("normal typing did not continue: %q", got.input.Value())
	}
}

func TestSelectionKeyClipboardErrorIsNonFatalStatus(t *testing.T) {
	model := New(nil)
	model.input.SetValue("copy composer")
	model.composerSelection = selection.OffsetRange{Anchor: 0, Focus: 13, Active: true}
	model.clipboardWrite = func(string) error {
		return errors.New("clipboard denied")
	}

	updated, _ := model.Update(tea.KeyPressMsg{Code: 'c'})
	got := updated.(Model)

	if !strings.Contains(got.status, "clipboard failed") {
		t.Fatalf("status = %q", got.status)
	}
}
