package components

import (
	"strings"

	"charm.land/lipgloss/v2"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

var attachmentStyle = lipgloss.NewStyle().
	Foreground(lipgloss.Color("252")).
	Padding(0, 2)

func AttachmentBar(width int, attachments []viewmodel.Attachment) string {
	if len(attachments) == 0 {
		return ""
	}
	labels := make([]string, 0, len(attachments))
	for _, attachment := range attachments {
		kind := strings.TrimSpace(attachment.Kind)
		if kind == "" {
			kind = "file"
		}
		labels = append(labels, "@"+attachment.Path+" ["+kind+"]")
	}
	return attachmentStyle.Width(max(1, width-1)).Render(strings.Join(labels, "  "))
}
