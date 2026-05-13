package components

import (
	"strings"

	zone "github.com/lrstanley/bubblezone/v2"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

func OverlayListDialog(base string, width int, height int, dialog viewmodel.ListDialog, zones *zone.Manager) string {
	return overlayDialogBox(base, width, height, ListDialogBox(width, dialog, zones))
}

func OverlayPermissionDialog(base string, width int, height int, dialog viewmodel.PermissionDialog, zones *zone.Manager) string {
	return overlayDialogBox(base, width, height, PermissionDialogBox(width, dialog, zones))
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
