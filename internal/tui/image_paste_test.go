package tui

import (
	"context"
	"os"
	"path/filepath"
	"testing"

	tea "charm.land/bubbletea/v2"
	"github.com/pandelisz/gode/internal/godex"
	"github.com/pandelisz/gode/internal/tui/attachments"
)

func TestBracketedPasteImagePathAttachesImage(t *testing.T) {
	dir := t.TempDir()
	imagePath := filepath.Join(dir, "pasted.png")
	if err := os.WriteFile(imagePath, tinyPNG, 0o600); err != nil {
		t.Fatal(err)
	}
	model := New(&godex.App{Config: godex.Config{Workspace: dir, DataDir: dir}})

	got, _ := model.Update(tea.PasteMsg{Content: imagePath})
	updated := got.(Model)

	if len(updated.attachments) != 1 {
		t.Fatalf("attachments = %#v", updated.attachments)
	}
	if updated.attachments[0].Kind != attachments.KindImage || updated.attachments[0].Path != filepath.ToSlash(imagePath) {
		t.Fatalf("attachment = %#v", updated.attachments[0])
	}
}

func TestCtrlVPastesClipboardImage(t *testing.T) {
	dir := t.TempDir()
	imagePath := filepath.Join(dir, "clipboard.png")
	if err := os.WriteFile(imagePath, tinyPNG, 0o600); err != nil {
		t.Fatal(err)
	}
	model := New(&godex.App{Config: godex.Config{Workspace: dir, DataDir: dir}})
	model.imagePaste = func(context.Context, string) (attachments.Attachment, error) {
		return attachments.New(imagePath), nil
	}

	got, cmd := model.Update(tea.KeyPressMsg(tea.Key{Code: 'v', Mod: tea.ModCtrl}))
	if cmd != nil {
		msg := cmd()
		got, _ = got.Update(msg)
	}
	updated := got.(Model)

	if len(updated.attachments) != 1 {
		t.Fatalf("attachments = %#v", updated.attachments)
	}
	if updated.status != "image attached" {
		t.Fatalf("status = %q", updated.status)
	}
}

var tinyPNG = []byte{
	0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a,
	0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
	0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
	0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
	0x89, 0x00, 0x00, 0x00, 0x0a, 0x49, 0x44, 0x41,
	0x54, 0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00,
	0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00,
	0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae,
	0x42, 0x60, 0x82,
}
