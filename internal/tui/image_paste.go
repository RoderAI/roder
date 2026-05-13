package tui

import (
	"context"
	"os"
	"path/filepath"
	"strings"

	tea "charm.land/bubbletea/v2"
	"github.com/pandelisz/gode/internal/tui/attachments"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

type imagePasteDoneMsg struct {
	Attachment attachments.Attachment
	Err        error
}

func (m Model) pasteImageFromClipboard() tea.Cmd {
	dataDir := ".gode"
	if m.app != nil && m.app.Config.DataDir != "" {
		dataDir = m.app.Config.DataDir
	}
	paste := m.imagePaste
	if paste == nil {
		paste = attachments.PasteImageFromClipboard
	}
	return func() tea.Msg {
		attachment, err := paste(context.Background(), dataDir)
		return imagePasteDoneMsg{Attachment: attachment, Err: err}
	}
}

func (m Model) pastedImagePath(content string) (attachments.Attachment, bool) {
	path := strings.Trim(strings.TrimSpace(content), "\"'")
	if path == "" {
		return attachments.Attachment{}, false
	}
	if !filepath.IsAbs(path) {
		if m.app == nil {
			return attachments.Attachment{}, false
		}
		path = filepath.Join(m.app.Config.Workspace, path)
	}
	info, err := os.Stat(path)
	if err != nil || info.IsDir() {
		return attachments.Attachment{}, false
	}
	attachment := attachments.New(path)
	return attachment, attachment.Kind == attachments.KindImage
}

func (m *Model) attachImage(attachment attachments.Attachment) {
	m.attachments = append(m.attachments, attachment)
	m.status = "image attached"
}

func (m *Model) failImagePaste(err error) {
	if err == nil {
		return
	}
	m.addMessage(viewmodel.RoleError, "attachments", err.Error())
	m.status = "image paste failed - ctrl+l errors"
}
