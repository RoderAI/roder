package components

import (
	"strconv"
	"strings"

	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

func OverlayRemoteDialog(base string, width int, height int, dialog viewmodel.RemoteDialog) string {
	return overlayDialogBox(base, width, height, RemoteDialogBox(width, height, dialog))
}

func RemoteDialogBox(width int, height int, dialog viewmodel.RemoteDialog) string {
	dialogWidth := min(96, max(50, width-8))
	contentWidth := max(34, dialogWidth-6)
	maxQRHeight := max(4, min(18, height/3))
	lines := []string{
		settingsTitleStyle.Render(firstNonEmpty(dialog.Title, "Remote Control")),
		"",
		settingsLabelValue(contentWidth, "Status", remoteStatus(dialog)),
		settingsLabelValue(contentWidth, "Clients", intString(dialog.ConnectedClients)),
	}
	if dialog.TokenPreview != "" {
		lines = append(lines, settingsLabelValue(contentWidth, "Token", dialog.TokenPreview))
	}
	if len(dialog.URLs) > 0 {
		lines = append(lines, "", settingsSelectedStyle.Render("Connect URLs"))
		for _, remoteURL := range dialog.URLs {
			lines = append(lines, settingsItemStyle.Render(truncateCell(remoteURL, contentWidth)))
		}
	}
	if dialog.AuthHeaderHint != "" {
		lines = append(lines, "", settingsDescriptionStyle.Render(truncateCell(dialog.AuthHeaderHint, contentWidth)))
	}
	if dialog.SubprotocolHint != "" {
		lines = append(lines, settingsDescriptionStyle.Render(truncateCell(dialog.SubprotocolHint, contentWidth)))
	}
	if dialog.Warning != "" {
		lines = append(lines, "", settingsErrorStyle.Render(truncateCell(dialog.Warning, contentWidth)))
	}
	if dialog.QR != "" {
		lines = append(lines, "")
		lines = append(lines, cropBlock(dialog.QR, contentWidth, maxQRHeight)...)
	}
	if dialog.Error != "" {
		lines = append(lines, "", settingsErrorStyle.Render(truncateCell(dialog.Error, contentWidth)))
	}
	if dialog.Help != "" {
		lines = append(lines, "", settingsHelpStyle.Render(dialog.Help))
	}
	return settingsBoxStyle.Width(dialogWidth).Render(strings.Join(lines, "\n"))
}

func remoteStatus(dialog viewmodel.RemoteDialog) string {
	if dialog.Running {
		return "running"
	}
	return "stopped"
}

func cropBlock(block string, width int, height int) []string {
	raw := strings.Split(block, "\n")
	if len(raw) > height {
		raw = raw[:height]
		raw = append(raw, settingsDescriptionStyle.Render("... qr cropped to fit terminal"))
	}
	lines := make([]string, 0, len(raw))
	for _, line := range raw {
		lines = append(lines, truncateCell(line, width))
	}
	return lines
}

func firstNonEmpty(values ...string) string {
	for _, value := range values {
		if strings.TrimSpace(value) != "" {
			return value
		}
	}
	return ""
}

func intString(value int) string {
	return strconv.Itoa(value)
}
