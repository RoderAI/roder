package components

import (
	"strings"

	"charm.land/lipgloss/v2"
	zone "github.com/lrstanley/bubblezone/v2"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

var (
	inlineListStyle = lipgloss.NewStyle().
			Padding(0, 2)
	inlineListLabelStyle = lipgloss.NewStyle().
				Foreground(lipgloss.Color("231"))
	inlineListSelectedLabelStyle = lipgloss.NewStyle().
					Bold(true).
					Foreground(lipgloss.Color("152"))
	inlineListDescriptionStyle = lipgloss.NewStyle().
					Foreground(lipgloss.Color("244"))
)

func OverlayListDialog(base string, width int, height int, dialog viewmodel.ListDialog, zones *zone.Manager) string {
	return overlayDialogBox(base, width, height, ListDialogBox(width, dialog, zones))
}

func OverlayPermissionDialog(base string, width int, height int, dialog viewmodel.PermissionDialog, zones *zone.Manager) string {
	return overlayDialogBox(base, width, height, PermissionDialogBox(width, dialog, zones))
}

func OverlayConfirmDialog(base string, width int, height int, dialog viewmodel.ConfirmDialog) string {
	return overlayDialogBox(base, width, height, ConfirmDialogBox(width, dialog))
}

func InlineListDialogHeight(dialog *viewmodel.ListDialog) int {
	if dialog == nil || len(dialog.Items) == 0 {
		return 0
	}
	height := min(maxSlashListRows, len(dialog.Items))
	if dialog.Error != "" {
		height++
	}
	return height
}

func InlineListDialog(width int, dialog viewmodel.ListDialog, zones *zone.Manager) string {
	contentWidth := max(20, width-4)
	rows := dialog.Items
	if len(rows) > maxSlashListRows {
		rows = rows[:maxSlashListRows]
	}
	lines := make([]string, 0, len(rows)+1)
	for _, item := range rows {
		labelStyle := inlineListLabelStyle
		prefix := "  "
		if item.Selected {
			prefix = "> "
			labelStyle = inlineListSelectedLabelStyle
		}
		labelWidth := min(22, max(8, contentWidth/3))
		label := labelStyle.Render(truncateCell(item.Label, labelWidth))
		descWidth := max(1, contentWidth-lipgloss.Width(prefix)-labelWidth-2)
		desc := inlineListDescriptionStyle.Render(truncateCell(item.Description, descWidth))
		line := prefix + padCell(label, labelWidth) + "  " + desc
		if zones != nil {
			line = zones.Mark(viewmodel.DialogItemZoneID(dialog.Kind, item.ID), line)
		}
		lines = append(lines, line)
	}
	if dialog.Error != "" {
		lines = append(lines, settingsErrorStyle.Render(truncateCell(dialog.Error, contentWidth)))
	}
	return inlineListStyle.Width(width).Render(strings.Join(lines, "\n"))
}

func ListDialogBox(width int, dialog viewmodel.ListDialog, zones *zone.Manager) string {
	dialogWidth := min(78, max(44, width-8))
	contentWidth := max(30, dialogWidth-6)
	lines := []string{
		settingsTitleStyle.Render(dialog.Title),
		"",
	}
	if len(dialog.Items) == 0 {
		lines = append(lines, settingsDescriptionStyle.Render("No items available."))
	}
	for _, item := range dialog.Items {
		prefix := "  "
		style := settingsItemStyle
		if item.Selected {
			prefix = "> "
			style = settingsSelectedStyle
		}
		main := prefix + settingsLabelValue(contentWidth-2, item.Label, item.Value)
		if zones != nil {
			main = zones.Mark(viewmodel.DialogItemZoneID(dialog.Kind, item.ID), style.Render(main))
		} else {
			main = style.Render(main)
		}
		lines = append(lines, main)
		if item.Description != "" {
			lines = append(lines, "  "+settingsDescriptionStyle.Render(truncateCell(item.Description, contentWidth-2)))
		}
	}
	if dialog.Error != "" {
		lines = append(lines, "", settingsErrorStyle.Render(truncateCell(dialog.Error, contentWidth)))
	}
	if dialog.Help != "" {
		lines = append(lines, "", settingsHelpStyle.Render(dialog.Help))
	}
	return settingsBoxStyle.Width(dialogWidth).Render(strings.Join(lines, "\n"))
}

const maxSlashListRows = 8

func padCell(text string, width int) string {
	return text + strings.Repeat(" ", max(0, width-lipgloss.Width(text)))
}

func PermissionDialogBox(width int, dialog viewmodel.PermissionDialog, zones *zone.Manager) string {
	dialogWidth := min(78, max(44, width-8))
	contentWidth := max(30, dialogWidth-6)
	lines := []string{
		settingsTitleStyle.Render(dialog.Title),
		"",
	}
	if len(dialog.Requests) == 0 {
		lines = append(lines, settingsDescriptionStyle.Render("No pending permissions."))
	}
	for _, req := range dialog.Requests {
		prefix := "  "
		style := settingsItemStyle
		if req.Selected {
			prefix = "> "
			style = settingsSelectedStyle
		}
		main := prefix + settingsLabelValue(contentWidth-2, req.Tool, req.Action)
		if zones != nil {
			main = zones.Mark(viewmodel.DialogItemZoneID("permissions", req.ID), style.Render(main))
		} else {
			main = style.Render(main)
		}
		lines = append(lines, main)
		if req.Input != "" {
			lines = append(lines, "  "+settingsDescriptionStyle.Render(truncateCell(req.Input, contentWidth-2)))
		}
	}
	if dialog.Error != "" {
		lines = append(lines, "", settingsErrorStyle.Render(truncateCell(dialog.Error, contentWidth)))
	}
	if dialog.Help != "" {
		lines = append(lines, "", settingsHelpStyle.Render(dialog.Help))
	}
	return settingsBoxStyle.Width(dialogWidth).Render(strings.Join(lines, "\n"))
}

func ConfirmDialogBox(width int, dialog viewmodel.ConfirmDialog) string {
	dialogWidth := min(60, max(38, width-12))
	contentWidth := max(24, dialogWidth-6)
	confirm := strings.TrimSpace(dialog.ConfirmLabel)
	if confirm == "" {
		confirm = "Yes"
	}
	cancel := strings.TrimSpace(dialog.CancelLabel)
	if cancel == "" {
		cancel = "No"
	}
	help := strings.TrimSpace(dialog.Help)
	if help == "" {
		help = "enter quit  right/esc cancel"
	}
	lines := []string{
		settingsTitleStyle.Render(dialog.Title),
		"",
		settingsDescriptionStyle.Render(truncateCell(dialog.Message, contentWidth)),
		"",
		settingsSelectedStyle.Render("> "+confirm) + "  " + settingsItemStyle.Render(cancel),
		"",
		settingsHelpStyle.Render(help),
	}
	return settingsBoxStyle.Width(dialogWidth).Render(strings.Join(lines, "\n"))
}

func overlayDialogBox(base string, width int, height int, box string) string {
	baseLines := padLines(strings.Split(base, "\n"), width, height)
	boxLines := strings.Split(box, "\n")
	boxHeight := len(boxLines)
	boxWidth := maxLineWidth(boxLines)
	startY := clamp((height-boxHeight)/2, 0, max(0, height-1))
	startX := max(0, (width-boxWidth)/2)

	for i, line := range boxLines {
		y := startY + i
		if y >= len(baseLines) {
			break
		}
		baseLines[y] = strings.Repeat(" ", startX) + line
		baseLines[y] = padLine(baseLines[y], width)
	}
	return strings.Join(baseLines, "\n")
}
