package tui

import (
	"fmt"

	"github.com/pandelisz/gode/internal/godex"
)

func defaultContextLeft(app *godex.App) string {
	if app == nil {
		return "ctx --%"
	}
	return "ctx 100%"
}

func formatContextLeft(usedPercent float64) string {
	left := 100 - int(usedPercent+0.5)
	left = clamp(left, 0, 100)
	return fmt.Sprintf("ctx %d%%", left)
}
