package components

import (
	"strings"

	"charm.land/lipgloss/v2"
	zone "github.com/lrstanley/bubblezone/v2"
	"github.com/pandelisz/gode/internal/tui/viewmodel"
)

var (
	settingsBackdropStyle    lipgloss.Style
	settingsBoxStyle         lipgloss.Style
	settingsTitleStyle       lipgloss.Style
	settingsItemStyle        lipgloss.Style
	settingsSelectedStyle    lipgloss.Style
	settingsDescriptionStyle lipgloss.Style
	settingsValueStyle       lipgloss.Style
	settingsHelpStyle        lipgloss.Style
	settingsErrorStyle       lipgloss.Style
)

func resetSettingsStyles() {
	settingsBackdropStyle = lipgloss.NewStyle().
		Foreground(ThemeColor(ColorText))
	settingsBoxStyle = lipgloss.NewStyle().
		Border(lipgloss.RoundedBorder()).
		BorderForeground(ThemeColor(ColorDialog)).
		Padding(1, 2)
	settingsTitleStyle = lipgloss.NewStyle().
		Bold(true).
		Foreground(ThemeColor(ColorTextStrong))
	settingsItemStyle = lipgloss.NewStyle().
		Foreground(ThemeColor(ColorText))
	settingsSelectedStyle = lipgloss.NewStyle().
		Bold(true).
		Foreground(ThemeColor(ColorTextStrong))
	settingsDescriptionStyle = lipgloss.NewStyle().
		Foreground(ThemeColor(ColorMuted))
	settingsValueStyle = lipgloss.NewStyle().
		Foreground(ThemeColor(ColorValue))
	settingsHelpStyle = lipgloss.NewStyle().
		Foreground(ThemeColor(ColorMuted))
	settingsErrorStyle = lipgloss.NewStyle().
		Foreground(ThemeColor(ColorError))
}

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
	return overlayDialogBox(base, width, height, box)
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
	case viewmodel.SettingsScreenMemories:
		return memorySettingsContent(width, settings.Memory, zones)
	case viewmodel.SettingsScreenSkills:
		return skillSettingsContent(width, settings.Skills, zones)
	case viewmodel.SettingsScreenSkillRecs:
		return recommendedSkillSettingsContent(width, settings.RecommendedSkills, zones)
	case viewmodel.SettingsScreenSkillInstall:
		return installSkillSettingsContent(width, settings.InstallPrompt)
	default:
		return menuSettingsContent(width, settings.MenuItems, zones)
	}
}

func menuSettingsContent(width int, items []viewmodel.SettingsMenuItem, zones *zone.Manager) []string {
	lines := make([]string, 0, len(items)*2)
	showDescriptions := len(items) <= 7
	for _, item := range items {
		prefix := "  "
		style := settingsItemStyle
		if item.Selected {
			prefix = "> "
			style = settingsSelectedStyle
		}
		main := prefix + settingsLabelValue(width-2, item.Label, item.Value)
		block := markSettingsZone(zones, viewmodel.SettingsMenuItemZoneID(item.ID), style.Render(main))
		if showDescriptions && item.Selected && item.Description != "" {
			desc := "  " + settingsDescriptionStyle.Render(truncateCell(item.Description, width-2))
			block += "\n" + desc
		}
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
		if model.Provider != "" {
			value = model.Provider + "/" + model.ID
		}
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

func memorySettingsContent(width int, state viewmodel.SettingsMemoryState, zones *zone.Manager) []string {
	lines := make([]string, 0, len(state.Rows)*2)
	for _, row := range state.Rows {
		prefix := "  "
		style := settingsItemStyle
		if row.Selected {
			prefix = "> "
			style = settingsSelectedStyle
		}
		main := prefix + settingsLabelValue(width-2, row.Label, row.Value)
		block := markSettingsZone(zones, viewmodel.SettingsMemoryZoneID(row.ID), style.Render(main))
		if row.Selected && row.Description != "" {
			block += "\n" + "  " + settingsDescriptionStyle.Render(truncateCell(row.Description, width-2))
		}
		lines = append(lines, block)
	}
	return lines
}

func skillSettingsContent(width int, items []viewmodel.SettingsSkillItem, zones *zone.Manager) []string {
	if len(items) == 0 {
		return []string{settingsDescriptionStyle.Render("No skills installed. Press i to install one or r for recommended skills.")}
	}
	lines := make([]string, 0, len(items)*2)
	for _, item := range items {
		prefix := "  "
		style := settingsItemStyle
		if item.Selected {
			prefix = "> "
			style = settingsSelectedStyle
		}
		name := item.Name
		if name == "" {
			name = "diagnostic"
		}
		value := item.State
		if item.Scope != "" {
			value += " " + item.Scope
		}
		main := prefix + settingsLabelValue(width-2, name, value)
		detail := item.Description
		if detail == "" {
			detail = item.Diagnostic
		}
		if detail == "" {
			detail = item.Path
		}
		block := markSettingsZone(zones, viewmodel.SettingsSkillZoneID(item.Name), style.Render(main))
		if detail != "" {
			block += "\n" + "  " + settingsDescriptionStyle.Render(truncateCell(detail, width-2))
		}
		lines = append(lines, block)
	}
	return lines
}

func recommendedSkillSettingsContent(width int, items []viewmodel.SettingsRecommendedSkillItem, zones *zone.Manager) []string {
	lines := make([]string, 0, len(items)*2)
	for _, item := range items {
		prefix := "  "
		style := settingsItemStyle
		if item.Selected {
			prefix = "> "
			style = settingsSelectedStyle
		}
		main := prefix + settingsLabelValue(width-2, item.Name, item.State)
		block := markSettingsZone(zones, viewmodel.SettingsRecommendedSkillZoneID(item.Name), style.Render(main))
		if item.Source != "" {
			block += "\n" + "  " + settingsDescriptionStyle.Render(truncateCell(item.Source, width-2))
		}
		lines = append(lines, block)
	}
	return lines
}

func installSkillSettingsContent(width int, prompt viewmodel.SettingsInstallPrompt) []string {
	source := prompt.Source
	if source == "" {
		source = "pandelisz/gode@go-development"
		source = settingsDescriptionStyle.Render(source)
	}
	status := "ready"
	if prompt.Installing {
		status = "installing"
	}
	return []string{
		settingsLabelValue(width, "Source", source),
		settingsLabelValue(width, "Status", status),
		settingsDescriptionStyle.Render(truncateCell("Use a local path, git URL, or owner/repo@skill source.", width)),
	}
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
	case viewmodel.SettingsScreenMemories:
		return "space toggle  enter toggle  esc back  up/down navigate"
	case viewmodel.SettingsScreenSkills:
		return "space toggle  i install  r recommended  esc back"
	case viewmodel.SettingsScreenSkillRecs:
		return "a install missing  esc back  up/down navigate"
	case viewmodel.SettingsScreenSkillInstall:
		return "enter install  esc back  type source"
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
