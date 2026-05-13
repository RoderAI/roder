package tui

import (
	"context"

	tea "charm.land/bubbletea/v2"
	"github.com/pandelisz/gode/internal/godex"
)

func Run(ctx context.Context, app *godex.App) error {
	program := tea.NewProgram(New(app), tea.WithContext(ctx))
	_, err := program.Run()
	return err
}

func NewResume(app *godex.App) Model {
	model := New(app)
	model.openResumeSessions()
	return model
}

func RunResume(ctx context.Context, app *godex.App) error {
	program := tea.NewProgram(NewResume(app), tea.WithContext(ctx))
	_, err := program.Run()
	return err
}
