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

func NewSession(app *godex.App, sessionID string) (Model, error) {
	model := New(app)
	if sessionID == "" {
		return model, nil
	}
	if err := model.loadSessionMessages(sessionID); err != nil {
		return model, err
	}
	model.setCurrentSession(sessionID)
	model.status = "session loaded"
	return model, nil
}

func RunSession(ctx context.Context, app *godex.App, sessionID string) error {
	model, err := NewSession(app, sessionID)
	if err != nil {
		return err
	}
	program := tea.NewProgram(model, tea.WithContext(ctx))
	_, err = program.Run()
	return err
}
