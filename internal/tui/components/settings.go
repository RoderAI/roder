package components

import (
	"strings"

	"charm.land/lipgloss/v2"
	zone "github.com/lrstanley/bubblezone/v2"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

var (
	settingsBackdropStyle = lipgloss.NewStyle().
				Foreground(lipgloss.Color("252"))
	settingsBoxStyle = lipgloss.NewStyle().
				Border(lipgloss.RoundedBorder()).
				BorderForeground(lipgloss.Color("62")).
				Padding(1, 2)
	settingsTitleStyle = lipgloss.NewStyle().
				Bold(true).
				Foreground(lipgloss.Color("231"))
	settingsItemStyle = lipgloss.NewStyle().
				Foreground(lipgloss.Color("252"))
	settingsSelectedStyle = lipgloss.NewStyle().
				Bold(true).
				Foreground(lipgloss.Color("231"))
	settingsDescriptionStyle = lipgloss.NewStyle().
					Foreground(lipgloss.Color("244"))
	settingsValueStyle = lipgloss.NewStyle().
				Foreground(lipgloss.Color("111"))
	settingsHelpStyle = lipgloss.NewStyle().
				Foreground(lipgloss.Color("244"))
	settingsErrorStyle = lipgloss.NewStyle().
				Foreground(lipgloss.Color("203"))
)

func SettingsDialog(width int, height int, settings viewmodel.SettingsDialog, zones *zone.Manager) string {
	box := SettingsDialogBox(width, settings, zones)
	return settingsBackdropStyle.Render(lipgloss.Place(
		width,
		height,
		lipgloss.Center,
		lipgloss.Center,
		box,
		lipgloss.WithWhitespaceChars(" "),
	))
}

func OverlaySettingsDialog(base string, width int, height int, settings viewmodel.SettingsDialog, zones *zone.Manager) string {
	box := SettingsDialogBox(width, settings, zones)
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

func SettingsDialogBox(width int, settings viewmodel.SettingsDialog, zones *zone.Manager) string {
	dialogWidth := min(72, max(42, width-8))
	contentWidth := max(28, dialogWidth-6)
	lines := []string{
		settingsTitleStyle.Render(settingsTitle(settings)),
		"",
	}
	lines = append(lines, settingsContent(contentWidth, settings, zones)...)

	if settings.Error != "" {
		lines = append(lines, "", settingsErrorStyle.Render(truncateCell(settings.Error, contentWidth)))
	}
	lines = append(lines, "", settingsHelpStyle.Render(settingsHelp(settings.Screen)))

	return settingsBoxStyle.Width(dialogWidth).Render(strings.Join(lines, "\n"))
}

func settingsTitle(settings viewmodel.SettingsDialog) string {
	if settings.Title != "" {
		return settings.Title
	}
	return "Settings"
}

func settingsContent(width int, settings viewmodel.SettingsDialog, zones *zone.Manager) []string {
	switch settings.Screen {
	case viewmodel.SettingsScreenModels:
		return modelSettingsContent(width, settings.Models, zones)
	case viewmodel.SettingsScreenReasoning:
		return reasoningSettingsContent(width, settings.Reasoning, zones)
	case viewmodel.SettingsScreenConfig:
		return configSettingsContent(width, settings.ConfigRows)
	default:
		return menuSettingsContent(width, settings.MenuItems, zones)
	}
}

func menuSettingsContent(width int, items []viewmodel.SettingsMenuItem, zones *zone.Manager) []string {
	lines := make([]string, 0, len(items)*2)
	for _, item := range items {
		prefix := "  "
		style := settingsItemStyle
		if item.Selected {
			prefix = "> "
			style = settingsSelectedStyle
		}
		main := prefix + settingsLabelValue(width-2, item.Label, item.Value)
		desc := "  " + settingsDescriptionStyle.Render(truncateCell(item.Description, width-2))
		block := markSettingsZone(zones, viewmodel.SettingsMenuItemZoneID(item.ID), style.Render(main)) + "\n" + desc
		lines = append(lines, block)
	}
	return lines
}

func modelSettingsContent(width int, models []viewmodel.SettingsModelItem, zones *zone.Manager) []string {
	lines := make([]string, 0, len(models))
	for _, model := range models {
		prefix := "  "
		style := settingsItemStyle
		if model.Selected {
			prefix = "> "
			style = settingsSelectedStyle
		}

		name := model.DisplayName
		if name == "" {
			name = model.ID
		}
		value := model.ID
		if model.Current {
			value += " current"
		}
		main := prefix + settingsLabelValue(width-2, name, value)
		lines = append(lines, markSettingsZone(zones, viewmodel.SettingsModelZoneID(model.ID), style.Render(main)))
	}
	return lines
}

func reasoningSettingsContent(width int, items []viewmodel.SettingsReasoningItem, zones *zone.Manager) []string {
	lines := make([]string, 0, len(items)*2)
	for _, item := range items {
		prefix := "  "
		style := settingsItemStyle
		if item.Selected {
			prefix = "> "
			style = settingsSelectedStyle
		}
		value := item.Effort
		if item.Current {
			value += " current"
		}
		main := prefix + settingsLabelValue(width-2, item.Label, value)
		block := markSettingsZone(zones, viewmodel.SettingsReasoningZoneID(item.Effort), style.Render(main))
		if item.Description != "" {
			block += "\n" + "  " + settingsDescriptionStyle.Render(truncateCell(item.Description, width-2))
		}
		lines = append(lines, block)
	}
	return lines
}

func configSettingsContent(width int, rows []viewmodel.SettingsConfigRow) []string {
	lines := make([]string, 0, len(rows))
	for _, row := range rows {
		lines = append(lines, settingsLabelValue(width, row.Label, row.Value))
	}
	return lines
}

func settingsLabelValue(width int, label string, value string) string {
	if value == "" {
		return truncateCell(label, width)
	}
	label = truncateCell(label, max(1, width/2))
	valueWidth := max(1, width-lipgloss.Width(label)-2)
	value = truncateCell(value, valueWidth)
	gap := strings.Repeat(" ", max(1, width-lipgloss.Width(label)-lipgloss.Width(value)))
	return label + gap + settingsValueStyle.Render(value)
}

func settingsHelp(screen string) string {
	switch screen {
	case viewmodel.SettingsScreenModels:
		return "enter choose reasoning  esc back  up/down navigate  ctrl+p close"
	case viewmodel.SettingsScreenReasoning:
		return "enter save default  esc back  up/down navigate  ctrl+p close"
	case viewmodel.SettingsScreenConfig:
		return "esc back  ctrl+p close"
	default:
		return "enter open  esc close  up/down navigate"
	}
}

func markSettingsZone(zones *zone.Manager, id string, content string) string {
	if zones == nil {
		return content
	}
	return zones.Mark(id, content)
}

func padLines(lines []string, width int, height int) []string {
	out := make([]string, height)
	for i := range out {
		if i < len(lines) {
			out[i] = padLine(lines[i], width)
			continue
		}
		out[i] = strings.Repeat(" ", width)
	}
	return out
}

func padLine(line string, width int) string {
	if lineWidth := lipgloss.Width(line); lineWidth < width {
		return line + strings.Repeat(" ", width-lineWidth)
	}
	return line
}

func maxLineWidth(lines []string) int {
	widest := 0
	for _, line := range lines {
		widest = max(widest, lipgloss.Width(line))
	}
	return widest
}
