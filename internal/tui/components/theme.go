package components

import (
	"image/color"
	"os"
	"sync"

	"charm.land/lipgloss/v2"
)

type ColorRole string

const (
	ColorText        ColorRole = "text"
	ColorTextStrong  ColorRole = "text-strong"
	ColorMuted       ColorRole = "muted"
	ColorSubtle      ColorRole = "subtle"
	ColorAccent      ColorRole = "accent"
	ColorAccentSoft  ColorRole = "accent-soft"
	ColorTool        ColorRole = "tool"
	ColorToolRunning ColorRole = "tool-running"
	ColorError       ColorRole = "error"
	ColorErrorLabel  ColorRole = "error-label"
	ColorBorder      ColorRole = "border"
	ColorDialog      ColorRole = "dialog"
	ColorValue       ColorRole = "value"
	ColorSelectionFG ColorRole = "selection-fg"
	ColorSelectionBG ColorRole = "selection-bg"
)

type Theme struct {
	Dark   bool
	Colors map[ColorRole]string
}

var (
	themeMu      sync.RWMutex
	activeTheme  = ThemeForDarkBackground(true)
	themeVersion int
)

func DetectAndSetTheme() {
	SetTheme(DetectTheme())
}

func DetectTheme() Theme {
	return ThemeForDarkBackground(lipgloss.HasDarkBackground(os.Stdin, os.Stdout))
}

func SetTheme(theme Theme) {
	themeMu.Lock()
	activeTheme = theme.withDefaults()
	themeVersion++
	themeMu.Unlock()
	rebuildStyles()
}

func ThemeForDarkBackground(dark bool) Theme {
	if !dark {
		return Theme{
			Dark: false,
			Colors: map[ColorRole]string{
				ColorText:        "16",
				ColorTextStrong:  "16",
				ColorMuted:       "240",
				ColorSubtle:      "245",
				ColorAccent:      "198",
				ColorAccentSoft:  "96",
				ColorTool:        "172",
				ColorToolRunning: "25",
				ColorError:       "160",
				ColorErrorLabel:  "231",
				ColorBorder:      "240",
				ColorDialog:      "62",
				ColorValue:       "25",
				ColorSelectionFG: "231",
				ColorSelectionBG: "198",
			},
		}
	}
	return Theme{
		Dark: true,
		Colors: map[ColorRole]string{
			ColorText:        "252",
			ColorTextStrong:  "231",
			ColorMuted:       "244",
			ColorSubtle:      "245",
			ColorAccent:      "212",
			ColorAccentSoft:  "183",
			ColorTool:        "214",
			ColorToolRunning: "75",
			ColorError:       "196",
			ColorErrorLabel:  "231",
			ColorBorder:      "244",
			ColorDialog:      "62",
			ColorValue:       "111",
			ColorSelectionFG: "16",
			ColorSelectionBG: "212",
		},
	}
}

func ThemeColor(role ColorRole) color.Color {
	return lipgloss.Color(themeColor(role))
}

func ThemeSelectionStyle() lipgloss.Style {
	return lipgloss.NewStyle().
		Foreground(ThemeColor(ColorSelectionFG)).
		Background(ThemeColor(ColorSelectionBG))
}

func ThemeVersion() int {
	themeMu.RLock()
	defer themeMu.RUnlock()
	return themeVersion
}

func themeColor(role ColorRole) string {
	themeMu.RLock()
	defer themeMu.RUnlock()
	if color := activeTheme.Colors[role]; color != "" {
		return color
	}
	return ThemeForDarkBackground(activeTheme.Dark).Colors[role]
}

func (theme Theme) withDefaults() Theme {
	defaults := ThemeForDarkBackground(theme.Dark)
	if theme.Colors == nil {
		return defaults
	}
	colors := make(map[ColorRole]string, len(defaults.Colors))
	for role, color := range defaults.Colors {
		colors[role] = color
		if theme.Colors[role] != "" {
			colors[role] = theme.Colors[role]
		}
	}
	return Theme{Dark: theme.Dark, Colors: colors}
}

func rebuildStyles() {
	resetAttachmentStyles()
	resetComposerStyles()
	resetDialogStyles()
	resetErrorConsoleStyles()
	resetFooterStyles()
	resetHeaderStyles()
	resetQueuedPromptStyles()
	resetReasoningSummaryStyles()
	resetSettingsStyles()
	resetTranscriptStyles()
}

func init() {
	rebuildStyles()
}
